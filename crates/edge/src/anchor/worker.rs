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
//! The [`Anchor`] run loop: drain ready batches.

use super::{Anchor, AnchorError, ChainConfig, LibcpsTarget};
use crate::batch_store::{BatchStore, StoredBatch};
use crate::chain::ChainRegistry;
use crate::observability::metrics::{
    ANCHOR_CONFIRMATION_TOTAL, ANCHOR_FAILURE_TOTAL, ANCHOR_RETRY_TOTAL, ANCHOR_SUBMISSION_TOTAL,
};
use crate::shutdown::ShutdownSignal;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// How long an in-flight transaction may keep running after shutdown was
/// requested. Anything abandoned is simply submitted again after restart.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);

/// Safety-net poll interval for ready batches if a wake-up is ever missed.
const IDLE_POLL: Duration = Duration::from_secs(30);

/// Most ready batches held in memory at once. Anchoring is serial and each
/// confirmed batch leaves the store, so the next page is simply the next
/// oldest batches; recovery memory stays bounded regardless of backlog size.
const READY_PAGE: usize = 4;

/// Outcome of a retried operation.
enum Retried<T> {
    /// The operation succeeded.
    Done(T),
    /// Shutdown was requested before it succeeded.
    Shutdown,
}

impl Anchor {
    /// Keep anchoring ready batches until `shutdown`.
    ///
    /// Batches left over from a previous run are submitted first (same ids,
    /// same bytes); new ones are picked up when `ready` is notified. Transient
    /// errors are retried with back-off; a permanent error stops the anchor and is
    /// returned, leaving the batch durably stored.
    pub async fn run(
        self,
        ready: Arc<Notify>,
        mut shutdown: ShutdownSignal,
    ) -> Result<(), AnchorError> {
        loop {
            let batches = match self
                .retry_transient("loading ready batches", &mut shutdown, || async {
                    Ok(self.store.ready_batches(READY_PAGE).await?)
                })
                .await?
            {
                Retried::Done(batches) => batches,
                Retried::Shutdown => return Ok(()),
            };

            if batches.is_empty() {
                tokio::select! {
                    _ = shutdown.recv() => return Ok(()),
                    _ = ready.notified() => {}
                    _ = tokio::time::sleep(IDLE_POLL) => {}
                }
                continue;
            }

            for batch in batches {
                if !self.anchor_one(&batch, &mut shutdown).await? {
                    return Ok(());
                }
            }
        }
    }

    /// Anchor `batch` and remove it locally. Returns `false` on shutdown.
    async fn anchor_one(
        &self,
        batch: &StoredBatch,
        shutdown: &mut ShutdownSignal,
    ) -> Result<bool, AnchorError> {
        let confirmation = match self
            .retry_transient("batch submission", shutdown, || async {
                metrics::counter!(ANCHOR_SUBMISSION_TOTAL).increment(1);
                tracing::info!(
                    batch_id = %batch.id,
                    messages = batch.message_count,
                    payload_bytes = batch.payload.len(),
                    "submitting batch"
                );
                self.submit_batch(batch).await
            })
            .await?
        {
            Retried::Done(confirmation) => confirmation,
            Retried::Shutdown => return Ok(false),
        };
        tracing::info!(
            batch_id = %batch.id,
            extrinsic = %confirmation.extrinsic_hash,
            block = %confirmation.block_hash,
            "batch finalized"
        );
        metrics::counter!(ANCHOR_CONFIRMATION_TOTAL).increment(1);

        match self
            .retry_transient("confirming batch", shutdown, || async {
                Ok(self.store.confirm_batch(batch.id).await?)
            })
            .await?
        {
            Retried::Done(()) => Ok(true),
            Retried::Shutdown => Ok(false),
        }
    }

    /// Run `op` until it succeeds, backing off on transient errors.
    ///
    /// A permanent error is counted, logged and returned. When shutdown is
    /// requested the attempt in flight gets [`SHUTDOWN_DRAIN`] to finish.
    async fn retry_transient<T, F, Fut>(
        &self,
        what: &str,
        shutdown: &mut ShutdownSignal,
        mut op: F,
    ) -> Result<Retried<T>, AnchorError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, AnchorError>>,
    {
        let mut attempt = 0u32;
        loop {
            if shutdown.is_triggered() {
                return Ok(Retried::Shutdown);
            }
            let mut stop = shutdown.clone();
            let result = tokio::select! {
                result = op() => result,
                _ = async {
                    stop.recv().await;
                    tokio::time::sleep(SHUTDOWN_DRAIN).await;
                } => return Ok(Retried::Shutdown),
            };
            match result {
                Ok(value) => return Ok(Retried::Done(value)),
                Err(error) if error.is_transient() => {
                    let delay = self.retry.delay(attempt);
                    attempt = attempt.saturating_add(1);
                    metrics::counter!(ANCHOR_RETRY_TOTAL).increment(1);
                    tracing::warn!(%error, what, attempt, ?delay, "retrying");
                    tokio::select! {
                        _ = shutdown.recv() => return Ok(Retried::Shutdown),
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
                Err(error) => {
                    metrics::counter!(ANCHOR_FAILURE_TOTAL).increment(1);
                    tracing::error!(%error, what, "permanent anchoring failure; anchor stopped");
                    return Err(error);
                }
            }
        }
    }
}

/// Obtain the shared chain connection and verify the anchoring account on it.
async fn connect_target(
    chains: &Arc<ChainRegistry>,
    chain: &ChainConfig,
) -> Result<LibcpsTarget, AnchorError> {
    LibcpsTarget::connect(
        chains.clone(),
        chain.endpoint.clone(),
        &chain.suri,
        chain.node,
    )
    .await
}

/// Connect to the chain (retrying transient failures), then run an
/// [`Anchor`] over `store` until `shutdown`.
///
/// Connecting happens here rather than at daemon startup so an unreachable
/// chain never blocks ingestion: messages and batches simply accumulate
/// durably until the chain is back.
pub async fn run_chain_anchor(
    chain: ChainConfig,
    chains: Arc<ChainRegistry>,
    store: Arc<dyn BatchStore>,
    ready: Arc<Notify>,
    mut shutdown: ShutdownSignal,
) {
    let mut attempt = 0u32;
    let target = loop {
        let result = tokio::select! {
            biased;
            _ = shutdown.recv() => return,
            result = connect_target(&chains, &chain) => result,
        };
        match result {
            Ok(target) => break target,
            Err(error) if error.is_transient() => {
                let delay = chain.retry.delay(attempt);
                attempt = attempt.saturating_add(1);
                metrics::counter!(ANCHOR_RETRY_TOTAL).increment(1);
                tracing::warn!(%error, attempt, ?delay, "chain connection failed; retrying");
                tokio::select! {
                    _ = shutdown.recv() => return,
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            Err(error) => {
                metrics::counter!(ANCHOR_FAILURE_TOTAL).increment(1);
                tracing::error!(%error, "cannot anchor: chain setup failed permanently");
                return;
            }
        }
    };
    tracing::info!(node = %chain.node, "connected to chain; anchor starting");

    let anchor = Anchor::new(store, Arc::new(target), chain.retry, chain.max_payload);
    if let Err(error) = anchor.run(ready, shutdown).await {
        tracing::error!(%error, "anchor stopped");
    }
}
