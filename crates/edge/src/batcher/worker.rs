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
//! The [`Batcher`] run loop.

use super::Batcher;
use crate::batch_store::Timestamp;
use crate::observability::metrics::{
    BATCHER_OLDEST_PENDING_AGE_SECONDS, BATCHER_PENDING_MESSAGES, BATCHER_READY_BATCHES,
};
use crate::shutdown::ShutdownSignal;
use std::time::Duration;

/// Back-off before retrying after a store failure.
const STORE_RETRY_DELAY: Duration = Duration::from_secs(1);

impl Batcher {
    /// Batch pending messages until `shutdown` fires, then flush the open
    /// batch.
    ///
    /// Pending messages already in the store (e.g. after a restart) are
    /// processed first. The loop sleeps until the oldest message's deadline
    /// or until it is woken by a new message.
    pub async fn run(self, mut shutdown: ShutdownSignal) {
        tracing::info!(
            max_batch_age_secs = self.config.max_batch_age_secs,
            max_batch_size = self.config.max_batch_size,
            max_messages = self.config.max_messages,
            compression = %self.config.compression,
            "batcher started"
        );
        loop {
            let next = match self.flush_due(Timestamp::now()).await {
                Ok(next) => next,
                Err(error) => {
                    tracing::error!(%error, "batching failed; retrying");
                    Some(STORE_RETRY_DELAY)
                }
            };
            self.report_gauges().await;

            let deadline = async {
                match next {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = shutdown.recv() => break,
                _ = self.wake.notified() => {}
                _ = deadline => {}
            }
        }

        if let Err(error) = self.flush_all().await {
            tracing::error!(%error, "final flush failed; messages stay pending");
        }
        self.report_gauges().await;
        tracing::info!("batcher stopped");
    }

    /// Publish queue-depth gauges.
    async fn report_gauges(&self) {
        let Ok(counts) = self.store.counts().await else {
            return;
        };
        metrics::gauge!(BATCHER_PENDING_MESSAGES).set(counts.pending_messages as f64);
        metrics::gauge!(BATCHER_READY_BATCHES).set(counts.ready_batches as f64);
        let oldest = self.store.pending_messages(1).await.ok();
        let age = oldest
            .and_then(|messages| messages.into_iter().next())
            .map(|m| m.received_at.elapsed_until(Timestamp::now()).as_secs_f64())
            .unwrap_or(0.0);
        metrics::gauge!(BATCHER_OLDEST_PENDING_AGE_SECONDS).set(age);
    }
}
