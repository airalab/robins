///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2026 Robonomics Network <research@robonomics.network>
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
///////////////////////////////////////////////////////////////////////////////
//! The [`Batcher`]: turns durable pending messages into immutable payloads.
//!
//! The batcher owns *everything* about what gets anchored:
//!
//! ```text
//!   pending messages ─▶ select (count / soft size / age)
//!                    ─▶ SignedEnvelopeBatch ─▶ serialize ─▶ compress
//!                    ─▶ final size check ─▶ (split + retry) ─▶ ready batches
//! ```
//!
//! The persisted payload is exactly the serialized (and, if configured,
//! compressed) `SignedEnvelopeBatch`: no magic bytes, version byte or length
//! prefix. How to read it is described by CPS metadata instead. Once
//! committed, a batch is never rebuilt; the [`Anchor`](crate::anchor::Anchor)
//! only submits the stored bytes.
//!
//! Flush triggers: `max_messages` reached, soft raw-size threshold reached
//! (uncompressed mode only, where the final size is known exactly), or the
//! oldest message reaching `max_batch_age` measured from its persisted
//! receive time, so messages restored after a restart that are already past
//! their deadline are flushed immediately.

mod compression;
mod config;
mod worker;

pub use compression::Compression;
pub use config::{BatcherConfig, PAYLOAD_ENCODING, PAYLOAD_SCHEMA, PAYLOAD_SCHEMA_VERSION};

use crate::batch_store::{
    BatchStore, MessageId, PreparedBatch, StoreError, StoredMessage, Timestamp,
};
use crate::observability::metrics::{
    BATCHER_COMPRESSION_RATIO, BATCHER_FLUSH_TOTAL, BATCHER_PAYLOAD_BYTES,
    BATCHER_PAYLOAD_TOO_LARGE_TOTAL, BATCHER_SERIALIZED_BYTES, BATCHER_SPLIT_TOTAL,
};
use crate::protocol::{self, SignedEnvelope, SignedEnvelopeBatch};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// Why a batch was flushed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushReason {
    /// `max_messages` reached.
    Count,
    /// The soft raw-size threshold was reached.
    Size,
    /// The oldest message reached `max_batch_age`.
    Age,
    /// The daemon is shutting down and flushes the open batch.
    Shutdown,
}

impl FlushReason {
    /// Metric label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Size => "size",
            Self::Age => "age",
            Self::Shutdown => "shutdown",
        }
    }
}

/// Errors raised while preparing batches.
#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    /// A single message cannot fit `max_batch_size` even on its own, so
    /// splitting cannot help. This is permanent: retrying never succeeds.
    #[error(
        "message {message} cannot fit max_batch_size: serialized {serialized} B, \
         final payload {payload} B > {max} B (compression: {compression})"
    )]
    PayloadTooLarge {
        /// The offending message.
        message: MessageId,
        /// Size of the one-message batch before compression.
        serialized: usize,
        /// Size of the one-message batch after compression.
        payload: usize,
        /// The configured `max_batch_size`.
        max: usize,
        /// The configured compression.
        compression: Compression,
    },
    /// A persisted message no longer decodes as a `SignedEnvelope`.
    #[error("message {message} is not a valid envelope: {reason}")]
    InvalidEnvelope {
        /// The offending message.
        message: MessageId,
        /// Decoder error.
        reason: String,
    },
    /// The compressor failed.
    #[error("compression failed: {0}")]
    Compression(#[from] std::io::Error),
    /// The batch store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// A decoded pending message.
struct Decoded {
    id: MessageId,
    envelope: SignedEnvelope,
}

/// Result of packing a candidate set of messages.
#[derive(Default)]
struct Packed {
    /// Final batches (with their pre-compression size), in message order.
    batches: Vec<(PreparedBatch, usize)>,
    /// Messages that could not be batched.
    rejected: Vec<BatchError>,
    /// How many times an oversized batch was split.
    splits: usize,
}

/// Which leading pending messages form the next batch, and whether it is due.
struct Selection {
    /// Number of leading messages in the candidate batch.
    len: usize,
    /// Set when a flush trigger fired.
    reason: Option<FlushReason>,
}

/// Outcome of one batching step.
enum Step {
    /// Nothing is pending.
    Idle,
    /// Messages are pending but no trigger fired yet; due in this long.
    Wait(Duration),
    /// A batch run was committed.
    Flushed,
}

/// Collects pending messages into durable, size-bounded batches.
pub struct Batcher {
    store: Arc<dyn BatchStore>,
    config: BatcherConfig,
    wake: Arc<Notify>,
    ready: Option<Arc<Notify>>,
}

impl Batcher {
    /// Create a batcher over `store`.
    ///
    /// `wake` is signalled by whoever persists messages (see
    /// [`BatchSink`](crate::pipeline::BatchSink)); it is only a wake-up hint,
    /// the store is the source of truth.
    pub fn new(store: Arc<dyn BatchStore>, config: BatcherConfig, wake: Arc<Notify>) -> Self {
        Self {
            store,
            config,
            wake,
            ready: None,
        }
    }

    /// Signal `ready` whenever new ready batches were committed, so the anchor
    /// can pick them up without polling.
    pub fn with_ready_notify(mut self, ready: Arc<Notify>) -> Self {
        self.ready = Some(ready);
        self
    }

    /// Flush every batch that is due at `now`.
    ///
    /// Returns how long until the oldest remaining message becomes due, or
    /// `None` if nothing is pending.
    pub async fn flush_due(&self, now: Timestamp) -> Result<Option<Duration>, BatchError> {
        loop {
            match self.step(now, false).await? {
                Step::Flushed => continue,
                Step::Wait(remaining) => return Ok(Some(remaining)),
                Step::Idle => return Ok(None),
            }
        }
    }

    /// Flush everything pending regardless of triggers (graceful shutdown).
    pub async fn flush_all(&self) -> Result<(), BatchError> {
        while !matches!(self.step(Timestamp::now(), true).await?, Step::Idle) {}
        Ok(())
    }

    /// Choose the next candidate batch from `pending` (oldest first) and decide
    /// whether to flush it now.
    fn select(&self, pending: &[StoredMessage], now: Timestamp) -> Selection {
        let config = &self.config;
        // The exact final size is only known up front without compression;
        // with compression, oversize batches are handled by splitting.
        let soft_size = config.compression == Compression::None;
        let mut len = 0;
        let mut size = 0usize;
        let mut reason = None;

        for message in pending {
            if len == config.max_messages {
                reason = Some(FlushReason::Count);
                break;
            }
            if soft_size {
                let wire = message.envelope.len();
                let entry = 1 + prost::length_delimiter_len(wire) + wire;
                if len > 0 && size + entry > config.max_batch_size {
                    reason = Some(FlushReason::Size);
                    break;
                }
                size += entry;
            }
            len += 1;
        }

        if reason.is_none() && len == config.max_messages {
            reason = Some(FlushReason::Count);
        }
        if reason.is_none() {
            if let Some(oldest) = pending.first() {
                if oldest.received_at.elapsed_until(now) >= config.max_batch_age() {
                    reason = Some(FlushReason::Age);
                }
            }
        }
        Selection { len, reason }
    }

    /// Run one batching step against the store.
    async fn step(&self, now: Timestamp, force: bool) -> Result<Step, BatchError> {
        let pending = self
            .store
            .pending_messages(self.config.max_messages)
            .await?;
        let Some(oldest) = pending.first() else {
            return Ok(Step::Idle);
        };

        let selection = self.select(&pending, now);
        let reason = match selection.reason {
            Some(reason) => reason,
            None if force => FlushReason::Shutdown,
            None => {
                let age = oldest.received_at.elapsed_until(now);
                return Ok(Step::Wait(self.config.max_batch_age().saturating_sub(age)));
            }
        };

        let candidate = &pending[..selection.len];
        let consumed: Vec<MessageId> = candidate.iter().map(|m| m.sequence).collect();
        let mut decoded = Vec::with_capacity(candidate.len());
        let mut rejected = Vec::new();
        for message in candidate {
            match protocol::decode_envelope(&message.envelope) {
                Ok(envelope) => decoded.push(Decoded {
                    id: message.sequence,
                    envelope,
                }),
                Err(err) => rejected.push(BatchError::InvalidEnvelope {
                    message: message.sequence,
                    reason: err.to_string(),
                }),
            }
        }

        let mut packed = self.pack(&decoded)?;
        packed.rejected.extend(rejected);

        let prepared: Vec<PreparedBatch> = packed.batches.iter().map(|(b, _)| b.clone()).collect();
        // Rejected messages are consumed too: they can never succeed, so
        // keeping them pending would wedge the queue forever.
        let ids = self.store.commit_batches(&consumed, &prepared).await?;
        if let (Some(ready), false) = (&self.ready, ids.is_empty()) {
            ready.notify_one();
        }

        for error in &packed.rejected {
            if matches!(error, BatchError::PayloadTooLarge { .. }) {
                metrics::counter!(BATCHER_PAYLOAD_TOO_LARGE_TOTAL).increment(1);
            }
            tracing::error!(%error, "message dropped: cannot be batched");
        }
        metrics::counter!(BATCHER_FLUSH_TOTAL, "reason" => reason.as_str()).increment(1);
        metrics::counter!(BATCHER_SPLIT_TOTAL).increment(packed.splits as u64);
        for (id, (batch, serialized)) in ids.iter().zip(&packed.batches) {
            metrics::histogram!(BATCHER_SERIALIZED_BYTES).record(*serialized as f64);
            metrics::histogram!(BATCHER_PAYLOAD_BYTES).record(batch.payload.len() as f64);
            if *serialized > 0 {
                metrics::gauge!(BATCHER_COMPRESSION_RATIO)
                    .set(batch.payload.len() as f64 / *serialized as f64);
            }
            tracing::info!(
                batch_id = %id,
                messages = batch.message_count,
                compression = %self.config.compression,
                serialized_bytes = serialized,
                payload_bytes = batch.payload.len(),
                splits = packed.splits,
                reason = reason.as_str(),
                "batch ready"
            );
        }
        Ok(Step::Flushed)
    }

    /// Encode `messages` into the fewest batches that satisfy
    /// `max_batch_size`, preserving order and losing or duplicating nothing.
    fn pack(&self, messages: &[Decoded]) -> Result<Packed, BatchError> {
        let mut packed = Packed::default();
        if !messages.is_empty() {
            self.pack_into(messages, &mut packed)?;
        }
        Ok(packed)
    }

    /// Encode `messages`; if the final payload is too big, split in half and
    /// run the whole encoding path again on each half.
    fn pack_into(&self, messages: &[Decoded], out: &mut Packed) -> Result<(), BatchError> {
        let (serialized, payload) = self.encode(messages)?;
        if payload.len() <= self.config.max_batch_size {
            out.batches.push((
                PreparedBatch {
                    message_count: messages.len() as u32,
                    payload,
                },
                serialized,
            ));
            return Ok(());
        }
        if let [only] = messages {
            out.rejected.push(BatchError::PayloadTooLarge {
                message: only.id,
                serialized,
                payload: payload.len(),
                max: self.config.max_batch_size,
                compression: self.config.compression,
            });
            return Ok(());
        }
        out.splits += 1;
        let (left, right) = messages.split_at(messages.len() / 2);
        self.pack_into(left, out)?;
        self.pack_into(right, out)
    }

    /// Build, serialize and compress one batch; returns the pre-compression
    /// size and the final payload.
    fn encode(&self, messages: &[Decoded]) -> Result<(usize, Vec<u8>), BatchError> {
        let batch = SignedEnvelopeBatch {
            batch: messages.iter().map(|m| m.envelope.clone()).collect(),
        };
        let serialized = protocol::encode_envelope_batch(&batch);
        let payload = self.config.compression.compress(&serialized)?;
        Ok((serialized.len(), payload))
    }
}

#[cfg(test)]
mod tests;
