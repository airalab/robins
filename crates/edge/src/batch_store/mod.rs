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
//! Durable storage for the batching pipeline.
//!
//! A [`BatchStore`] holds the two durable queues that make the anchoring path
//! crash-safe:
//!
//! ```text
//!   push_message ──▶ pending_messages ──commit_batches──▶ ready_batches ──confirm_batch──▶ (gone)
//! ```
//!
//! - **Pending** messages are authorized envelopes not yet batched, kept in
//!   ingress order together with their original receive time.
//! - **Ready** batches are immutable, fully encoded payloads awaiting
//!   blockchain confirmation.
//!
//! The Pending → Ready transition ([`BatchStore::commit_batches`]) is atomic:
//! messages are never removed without the batches that replace them being
//! durably stored. [`RedbBatchStore`] is the initial embedded backend.

mod model;
mod redb;

pub use self::redb::RedbBatchStore;
pub use model::{
    BatchId, FormatDescriptor, MessageId, PreparedBatch, StoreCounts, StoreError, StoreResult,
    StoredBatch, StoredMessage, Timestamp,
};

use async_trait::async_trait;

/// Durable queue of pending messages and ready batches.
///
/// Implementations must make every method durable before returning: once
/// [`push_message`](Self::push_message) resolves, the message survives a crash.
#[async_trait]
pub trait BatchStore: Send + Sync {
    /// Durably append an authorized envelope (its exact wire bytes) to the
    /// pending queue and return its sequence number.
    async fn push_message(
        &self,
        received_at: Timestamp,
        envelope: Vec<u8>,
    ) -> StoreResult<MessageId>;

    /// Up to `limit` of the oldest pending messages, in ingress order.
    async fn pending_messages(&self, limit: usize) -> StoreResult<Vec<StoredMessage>>;

    /// Atomically replace `consumed` pending messages with `prepared` ready
    /// batches, returning the identifiers assigned to the new batches.
    ///
    /// `consumed` may contain messages that did not end up in any batch (for
    /// example a message that can never fit the size limit). If any consumed
    /// message is no longer pending, nothing is changed and
    /// [`StoreError::NotPending`] is returned.
    async fn commit_batches(
        &self,
        consumed: &[MessageId],
        prepared: &[PreparedBatch],
    ) -> StoreResult<Vec<BatchId>>;

    /// All ready batches, oldest first.
    async fn ready_batches(&self) -> StoreResult<Vec<StoredBatch>>;

    /// Durably remove a batch whose anchoring was confirmed.
    async fn confirm_batch(&self, id: BatchId) -> StoreResult<()>;

    /// Current queue sizes.
    async fn counts(&self) -> StoreResult<StoreCounts>;

    /// The payload format the stored batches were prepared with, if recorded.
    async fn format_descriptor(&self) -> StoreResult<Option<FormatDescriptor>>;

    /// Record the payload format used for newly prepared batches.
    async fn set_format_descriptor(&self, descriptor: &FormatDescriptor) -> StoreResult<()>;
}

/// What [`reconcile_format`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatOutcome {
    /// The stored descriptor already matched the configuration.
    Unchanged,
    /// No descriptor was recorded yet; the configured one is now stored.
    Recorded,
    /// The format changed while no ready batch needed the old one; the
    /// configured descriptor replaced `previous`.
    Updated {
        /// The descriptor that was stored before.
        previous: FormatDescriptor,
    },
}

/// A payload-format change that cannot be applied safely.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    /// Prepared batches exist that were encoded under a different format.
    /// Anchoring them under the new CPS metadata would mislabel them.
    #[error(
        "payload format changed from {stored:?} to {configured:?} while {ready_batches} \
         prepared batch(es) are still waiting to be anchored; restore the previous \
         [batcher] compression or let the anchor drain them first"
    )]
    Incompatible {
        /// Descriptor the ready batches were prepared with.
        stored: FormatDescriptor,
        /// Descriptor derived from the current configuration.
        configured: FormatDescriptor,
        /// Number of ready batches that depend on `stored`.
        ready_batches: u64,
    },
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Compare the configured payload format with the one recorded in `store`.
///
/// Ready batches are immutable and already encoded, so the format they were
/// prepared with must not be relabelled. A different configuration is
/// therefore refused while any ready batch exists; once the queue is empty
/// the new format is recorded and the CPS metadata can be reconciled.
/// Pending messages are format-agnostic raw envelopes and never block a change.
pub async fn reconcile_format(
    store: &dyn BatchStore,
    configured: &FormatDescriptor,
) -> Result<FormatOutcome, FormatError> {
    match store.format_descriptor().await? {
        Some(stored) if &stored == configured => Ok(FormatOutcome::Unchanged),
        None => {
            store.set_format_descriptor(configured).await?;
            Ok(FormatOutcome::Recorded)
        }
        Some(stored) => {
            let ready_batches = store.counts().await?.ready_batches;
            if ready_batches > 0 {
                return Err(FormatError::Incompatible {
                    stored,
                    configured: configured.clone(),
                    ready_batches,
                });
            }
            store.set_format_descriptor(configured).await?;
            Ok(FormatOutcome::Updated { previous: stored })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(compression: &str) -> FormatDescriptor {
        FormatDescriptor {
            schema: "SignedEnvelopeBatch".into(),
            schema_version: 1,
            encoding: "protobuf".into(),
            compression: compression.into(),
        }
    }

    fn store() -> (tempfile::TempDir, RedbBatchStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = RedbBatchStore::open(dir.path().join("b.redb")).unwrap();
        (dir, store)
    }

    async fn make_ready(store: &RedbBatchStore) {
        let id = store.push_message(Timestamp::now(), vec![1]).await.unwrap();
        let batch = PreparedBatch {
            message_count: 1,
            payload: vec![1, 2, 3],
        };
        store.commit_batches(&[id], &[batch]).await.unwrap();
    }

    #[tokio::test]
    async fn first_start_records_then_matches() {
        let (_dir, store) = store();
        assert_eq!(
            reconcile_format(&store, &descriptor("zstd")).await.unwrap(),
            FormatOutcome::Recorded
        );
        assert_eq!(
            reconcile_format(&store, &descriptor("zstd")).await.unwrap(),
            FormatOutcome::Unchanged
        );
    }

    #[tokio::test]
    async fn change_with_ready_batches_fails_safely_and_keeps_old_format() {
        let (_dir, store) = store();
        reconcile_format(&store, &descriptor("none")).await.unwrap();
        make_ready(&store).await;

        let err = reconcile_format(&store, &descriptor("zstd"))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FormatError::Incompatible {
                ready_batches: 1,
                ..
            }
        ));
        assert_eq!(
            store.format_descriptor().await.unwrap(),
            Some(descriptor("none"))
        );
    }

    #[tokio::test]
    async fn change_with_empty_ready_queue_updates_the_format() {
        let (_dir, store) = store();
        reconcile_format(&store, &descriptor("none")).await.unwrap();
        // Pending messages are format-agnostic and do not block the change.
        store.push_message(Timestamp::now(), vec![1]).await.unwrap();

        assert_eq!(
            reconcile_format(&store, &descriptor("zstd")).await.unwrap(),
            FormatOutcome::Updated {
                previous: descriptor("none")
            }
        );
        assert_eq!(
            store.format_descriptor().await.unwrap(),
            Some(descriptor("zstd"))
        );
    }
}
