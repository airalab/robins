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
//! Data model shared by every [`BatchStore`](super::BatchStore) backend.
//!
//! These types are deliberately plain: raw wire bytes plus a few integers, so
//! the persisted representation does not depend on the generated protobuf
//! types and a stored batch can be submitted without re-encoding anything.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A wall-clock instant, stored as milliseconds since the Unix epoch.
///
/// Persisted timestamps must survive restarts, so a monotonic clock cannot be
/// used: `max_batch_age` is measured against the original receive time even
/// after the process was down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The current wall-clock time.
    pub fn now() -> Self {
        Self::from(SystemTime::now())
    }

    /// Build a timestamp from Unix epoch milliseconds.
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Unix epoch milliseconds.
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// Time elapsed from `self` until `now`, saturating to zero if the wall
    /// clock went backwards.
    pub fn elapsed_until(self, now: Timestamp) -> Duration {
        Duration::from_millis(now.0.saturating_sub(self.0))
    }
}

impl From<SystemTime> for Timestamp {
    fn from(time: SystemTime) -> Self {
        let millis = time
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        Self(u64::try_from(millis).unwrap_or(u64::MAX))
    }
}

/// Identifier of a pending message: its ingress-ordered sequence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifier of a prepared batch; strictly increasing in creation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BatchId(pub u64);

impl fmt::Display for BatchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An authorized envelope waiting to be batched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMessage {
    /// Ingress-ordered sequence number.
    pub sequence: MessageId,
    /// When the gateway originally accepted the message.
    pub received_at: Timestamp,
    /// The exact wire bytes of the `SignedEnvelope`.
    pub envelope: Vec<u8>,
}

/// A fully prepared batch that has not been given an identifier yet.
///
/// The payload is final: the store persists it verbatim and the anchor
/// submits exactly these bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedBatch {
    /// Number of envelopes encoded in `payload`.
    pub message_count: u32,
    /// Serialized (and possibly compressed) `SignedEnvelopeBatch`.
    pub payload: Vec<u8>,
}

/// A persisted, immutable batch awaiting blockchain confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredBatch {
    /// Batch identifier, stable across retries and restarts.
    pub id: BatchId,
    /// When the batch was committed to the store.
    pub created_at: Timestamp,
    /// Number of envelopes encoded in `payload`.
    pub message_count: u32,
    /// The exact bytes passed to `CPS.set_payload`.
    pub payload: Vec<u8>,
}

/// Cheap occupancy counters, used for metrics and recovery logging.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreCounts {
    /// Messages waiting to be batched.
    pub pending_messages: u64,
    /// Batches waiting to be anchored.
    pub ready_batches: u64,
}

/// Description of how batch payloads are encoded.
///
/// It is persisted next to the batches so a restart with a different payload
/// format cannot silently relabel batches that were prepared under the old
/// one, and it is the single source for the CPS metadata that tells readers
/// how to interpret a payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatDescriptor {
    /// Protobuf message the payload decodes to (e.g. `SignedEnvelopeBatch`).
    pub schema: String,
    /// Version of `schema`.
    pub schema_version: u32,
    /// Serialization format (e.g. `protobuf`).
    pub encoding: String,
    /// Compression applied after serialization (`none` or `zstd`).
    pub compression: String,
}

/// Errors reported by a [`BatchStore`](super::BatchStore).
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The underlying database failed.
    #[error("batch store database error: {0}")]
    Database(String),
    /// Persisted data could not be decoded.
    #[error("batch store contains corrupt data: {0}")]
    Corrupt(String),
    /// A batch commit referenced a message that is no longer pending.
    #[error("message {0} is not pending")]
    NotPending(MessageId),
    /// A blocking storage task panicked or was cancelled.
    #[error("batch store task failed: {0}")]
    Task(String),
}

impl StoreError {
    /// Wrap any displayable database error.
    pub(crate) fn database(err: impl fmt::Display) -> Self {
        Self::Database(err.to_string())
    }
}

/// Result alias for batch store operations.
pub type StoreResult<T> = Result<T, StoreError>;
