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
//! The durable [`MessageSink`] feeding the batching pipeline.

use super::{AcceptedMessage, MessageSink, SinkError};
use crate::batch_store::{BatchStore, Timestamp};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Notify;

/// Persists every accepted envelope to the [`BatchStore`] before returning.
///
/// The store, not memory, is the source of truth: once [`deliver`] resolves
/// successfully the envelope survives a crash. The `wake` handle only tells
/// the [`Batcher`](crate::batcher::Batcher) that there is new work; a missed
/// wake-up is harmless because the batcher also re-reads the store on its own
/// deadline and at startup.
///
/// [`deliver`]: MessageSink::deliver
pub struct BatchSink {
    store: Arc<dyn BatchStore>,
    wake: Arc<Notify>,
}

impl BatchSink {
    /// Create a sink writing to `store` and waking the batcher through `wake`.
    pub fn new(store: Arc<dyn BatchStore>, wake: Arc<Notify>) -> Self {
        Self { store, wake }
    }
}

#[async_trait]
impl MessageSink for BatchSink {
    fn name(&self) -> &'static str {
        "batch"
    }

    async fn deliver(&self, message: Arc<AcceptedMessage>) -> Result<(), SinkError> {
        self.store
            .push_message(
                Timestamp::from(message.received_at),
                message.raw_envelope.to_vec(),
            )
            .await
            .map_err(|source| SinkError::Delivery {
                sink: "batch",
                source: Box::new(source),
            })?;
        self.wake.notify_one();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_store::RedbBatchStore;
    use crate::ingress::TransportMetadata;
    use crate::protocol::{
        encode_envelope, envelope_id, sign_message, SensorIdentity, SignOptions,
    };
    use bytes::Bytes;
    use std::time::{Duration, SystemTime};

    #[tokio::test]
    async fn persists_exact_bytes_and_receive_time_then_wakes_batcher() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RedbBatchStore::open(dir.path().join("b.redb")).unwrap());
        let wake = Arc::new(Notify::new());
        let sink = BatchSink::new(store.clone(), wake.clone());

        let identity = SensorIdentity::generate();
        let envelope = sign_message(&identity, b"hello", SignOptions::default());
        let raw = Bytes::from(encode_envelope(&envelope));
        let received_at = SystemTime::now() - Duration::from_secs(5);
        let message = Arc::new(AcceptedMessage {
            sensor_id: identity.sensor_id(),
            envelope,
            envelope_id: envelope_id(&raw),
            raw_envelope: raw.clone(),
            transport: TransportMetadata::http(None),
            received_at,
        });

        sink.deliver(message).await.unwrap();

        let pending = store.pending_messages(10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].envelope, raw.to_vec());
        assert_eq!(pending[0].received_at, Timestamp::from(received_at));
        // The permit is stored, so the wake-up is not lost.
        tokio::time::timeout(Duration::from_secs(1), wake.notified())
            .await
            .expect("batcher woken");
    }
}
