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
//! The [`Anchor`]: the blockchain transaction lifecycle for ready batches.
//!
//! ```text
//!   ready_batches ─▶ Anchor ─▶ CPS.set_payload(exact stored bytes) ─▶ finalized ─▶ confirm_batch
//! ```
//!
//! The anchor owns *only* the transaction lifecycle: submitting, retrying and
//! confirming. It never touches the CPS node metadata; keeping it in sync with
//! the payload format is the job of a separate service. It treats
//! [`StoredBatch::payload`] as immutable opaque bytes and never serializes,
//! compresses, splits or otherwise repairs a batch; the
//! [`Batcher`](crate::batcher::Batcher) already did all of that before the
//! batch became durable.
//!
//! Delivery is **at-least-once**: if the process crashes after a batch was
//! finalized on chain but before it was removed locally, the same batch (same
//! id, same bytes) is submitted again after restart.
//!
//! The chain is reached through the small [`CpsTarget`] trait so the lifecycle
//! can be tested without a node; [`LibcpsTarget`] is the real implementation.

mod config;
mod cps;
mod worker;

pub use config::{ChainConfig, RetryPolicy};
pub use cps::LibcpsTarget;
pub use worker::run_chain_anchor;

use crate::batch_store::{BatchStore, StoreError, StoredBatch};
use crate::chain::ChainError;
use async_trait::async_trait;
use std::sync::Arc;

/// Proof that a transaction was finalized and executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    /// Hash of the extrinsic.
    pub extrinsic_hash: String,
    /// Hash of the finalized block that includes it.
    pub block_hash: String,
}

/// Errors of the anchoring lifecycle.
#[derive(Debug, thiserror::Error)]
pub enum AnchorError {
    /// A failure that may go away on its own (RPC unavailable, connection
    /// reset, timeout, temporary node failure): retried with back-off.
    #[error("transient chain error: {0}")]
    Transient(String),
    /// A failure retrying cannot fix (oversized payload, missing
    /// authorization, invalid key or node, bad configuration).
    #[error("permanent anchoring error: {0}")]
    Permanent(String),
    /// The local batch store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl AnchorError {
    /// Whether the operation should be retried.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::Store(_))
    }
}

/// The CPS node an [`Anchor`] writes to. Every mutating call returns only
/// once the transaction is finalized and successfully executed.
#[async_trait]
pub trait CpsTarget: Send + Sync {
    /// Replace the node payload with exactly `payload`.
    async fn set_payload(&self, payload: &[u8]) -> Result<Confirmation, AnchorError>;
}

/// Submits persisted ready batches to a CPS node.
pub struct Anchor {
    store: Arc<dyn BatchStore>,
    target: Arc<dyn CpsTarget>,
    retry: RetryPolicy,
    max_payload: usize,
}

impl Anchor {
    /// Create an anchor for `target`.
    ///
    /// `max_payload` is only a defensive bound: an oversized stored batch is reported, never repaired.
    pub fn new(
        store: Arc<dyn BatchStore>,
        target: Arc<dyn CpsTarget>,
        retry: RetryPolicy,
        max_payload: usize,
    ) -> Self {
        Self {
            store,
            target,
            retry,
            max_payload,
        }
    }

    /// Submit one stored batch with exactly its stored bytes. Single attempt.
    pub async fn submit_batch(&self, batch: &StoredBatch) -> Result<Confirmation, AnchorError> {
        if batch.payload.len() > self.max_payload {
            return Err(AnchorError::Permanent(format!(
                "batch {} payload is {} bytes, above the {} byte limit",
                batch.id,
                batch.payload.len(),
                self.max_payload
            )));
        }
        self.target.set_payload(&batch.payload).await
    }
}

#[cfg(test)]
mod tests;

impl From<ChainError> for AnchorError {
    fn from(error: ChainError) -> Self {
        match error {
            ChainError::Transient(message) => Self::Transient(message),
            ChainError::Permanent(message) => Self::Permanent(message),
        }
    }
}
