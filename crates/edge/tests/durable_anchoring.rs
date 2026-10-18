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
//! Crash-recovery and end-to-end tests for durable batching and anchoring.
//!
//! The chain is replaced by a recording [`CpsTarget`], so these exercise the
//! real [`RedbBatchStore`], [`BatchSink`], [`Batcher`] and [`Anchor`] together,
//! including simulated process restarts (dropping every component and
//! reopening the database file) and the fully wired daemon.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use bytes::Bytes;
use edge::anchor::{Anchor, AnchorError, Confirmation, CpsTarget, RetryPolicy};
use edge::app::EdgeApp;
use edge::batch_store::{
    reconcile_format, BatchStore, FormatError, FormatOutcome, RedbBatchStore, Timestamp,
};
use edge::batcher::{Batcher, BatcherConfig, Compression};
use edge::config::{AuthMode, Config};
use edge::ingress::TransportMetadata;
use edge::pipeline::{AcceptedMessage, BatchSink, MessageSink};
use edge::protocol::{
    decode_envelope_batch, encode_envelope, envelope_id, sign_message, SensorIdentity, SignOptions,
    SignedEnvelope,
};
use edge::shutdown::ShutdownController;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

/// A recording CPS node.
#[derive(Default)]
struct RecordingTarget {
    payloads: Mutex<Vec<Vec<u8>>>,
    failures: Mutex<VecDeque<AnchorError>>,
}

impl RecordingTarget {
    fn payloads(&self) -> Vec<Vec<u8>> {
        self.payloads.lock().unwrap().clone()
    }
}

fn confirmation() -> Confirmation {
    Confirmation {
        extrinsic_hash: "0x01".into(),
        block_hash: "0x02".into(),
    }
}

#[async_trait]
impl CpsTarget for RecordingTarget {
    async fn set_payload(&self, payload: &[u8]) -> Result<Confirmation, AnchorError> {
        self.payloads.lock().unwrap().push(payload.to_vec());
        match self.failures.lock().unwrap().pop_front() {
            Some(error) => Err(error),
            None => Ok(confirmation()),
        }
    }
}

fn open(path: &Path) -> Arc<RedbBatchStore> {
    Arc::new(RedbBatchStore::open(path).expect("open store"))
}

fn batcher_config(compression: Compression) -> BatcherConfig {
    BatcherConfig {
        max_batch_age_secs: 30,
        max_batch_size: 8192,
        max_messages: 100,
        compression,
    }
}

fn accepted(envelope: &SignedEnvelope, received_at: SystemTime) -> Arc<AcceptedMessage> {
    let raw = Bytes::from(encode_envelope(envelope));
    let sensor = edge::protocol::verify_envelope(envelope)
        .expect("valid")
        .sensor_id;
    Arc::new(AcceptedMessage {
        sensor_id: sensor,
        envelope: envelope.clone(),
        envelope_id: envelope_id(&raw),
        raw_envelope: raw,
        transport: TransportMetadata::http(None),
        received_at,
    })
}

fn signed(tag: u8) -> SignedEnvelope {
    sign_message(
        &SensorIdentity::generate(),
        &[tag; 24],
        SignOptions::default(),
    )
}

fn anchor(store: &Arc<RedbBatchStore>, target: &Arc<RecordingTarget>) -> Anchor {
    Anchor::new(
        store.clone(),
        target.clone(),
        RetryPolicy {
            min: Duration::from_millis(1),
            max: Duration::from_millis(5),
        },
        8192,
    )
}

/// Run `anchor` until the ready queue is empty, then stop it.
async fn drain(anchor: Anchor, store: &Arc<RedbBatchStore>) {
    let controller = ShutdownController::new();
    let task = tokio::spawn(anchor.run(Arc::new(Notify::new()), controller.subscribe()));
    for _ in 0..300 {
        if store.counts().await.unwrap().ready_batches == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    controller.trigger();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn envelope_travels_from_pending_through_batch_to_exact_payload_on_chain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batcher.redb");
    let envelopes = [signed(1), signed(2), signed(3)];
    let compression = Compression::Zstd;
    let target = Arc::new(RecordingTarget::default());

    // 1. Accepted messages are persisted before anything batches them.
    {
        let store = open(&path);
        let sink = BatchSink::new(store.clone(), Arc::new(Notify::new()));
        for envelope in &envelopes {
            sink.deliver(accepted(envelope, SystemTime::now()))
                .await
                .unwrap();
        }
        assert_eq!(store.counts().await.unwrap().pending_messages, 3);
        // Crash: nothing was batched yet.
    }

    // 2. Restart: pending messages are recovered and batched.
    let prepared_payload = {
        let store = open(&path);
        assert_eq!(store.counts().await.unwrap().pending_messages, 3);
        let batcher = Batcher::new(
            store.clone(),
            batcher_config(compression),
            Arc::new(Notify::new()),
        );
        batcher.flush_all().await.unwrap();
        let ready = store.ready_batches(usize::MAX).await.unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(store.counts().await.unwrap().pending_messages, 0);
        ready[0].payload.clone()
        // Crash again: batch is ready but not yet anchored.
    };

    // 3. Restart: the anchor submits exactly the persisted bytes.
    let store = open(&path);
    let before = store.ready_batches(usize::MAX).await.unwrap();
    assert_eq!(
        before[0].payload, prepared_payload,
        "not rebuilt on restart"
    );
    drain(anchor(&store, &target), &store).await;

    assert_eq!(target.payloads(), vec![prepared_payload.clone()]);
    assert_eq!(store.counts().await.unwrap().ready_batches, 0);
    // What lands on chain is the compressed protobuf batch, nothing else.
    let serialized = compression.decompress(&prepared_payload).unwrap();
    assert_eq!(decode_envelope_batch(&serialized).unwrap().batch, envelopes);
}

#[tokio::test]
async fn crash_after_finalization_resubmits_the_same_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batcher.redb");
    let target = Arc::new(RecordingTarget::default());

    {
        let store = open(&path);
        BatchSink::new(store.clone(), Arc::new(Notify::new()))
            .deliver(accepted(&signed(9), SystemTime::now()))
            .await
            .unwrap();
        Batcher::new(
            store.clone(),
            batcher_config(Compression::None),
            Arc::new(Notify::new()),
        )
        .flush_all()
        .await
        .unwrap();

        // The chain accepted the batch, but the process died before cleanup:
        // simulated by submitting directly and never calling `confirm_batch`.
        let batch = store.ready_batches(usize::MAX).await.unwrap().remove(0);
        target.set_payload(&batch.payload).await.unwrap();
    }

    let store = open(&path);
    drain(anchor(&store, &target), &store).await;

    let payloads = target.payloads();
    assert_eq!(payloads.len(), 2, "at-least-once: submitted again");
    assert_eq!(payloads[0], payloads[1]);
    assert_eq!(store.counts().await.unwrap().ready_batches, 0);
}

#[tokio::test]
async fn restored_message_uses_its_original_receive_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batcher.redb");
    let received_at = SystemTime::now() - Duration::from_secs(120);
    {
        let store = open(&path);
        BatchSink::new(store.clone(), Arc::new(Notify::new()))
            .deliver(accepted(&signed(4), received_at))
            .await
            .unwrap();
    }

    // Restart two minutes later with a 30 s `max_batch_age`: due immediately.
    let store = open(&path);
    let batcher = Batcher::new(
        store.clone(),
        batcher_config(Compression::Zstd),
        Arc::new(Notify::new()),
    );
    assert_eq!(batcher.flush_due(Timestamp::now()).await.unwrap(), None);
    assert_eq!(store.counts().await.unwrap().ready_batches, 1);
}

#[tokio::test]
async fn format_change_with_ready_batches_fails_until_the_queue_is_drained() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batcher.redb");
    let none = batcher_config(Compression::None).format_descriptor();
    let zstd = batcher_config(Compression::Zstd).format_descriptor();
    let target = Arc::new(RecordingTarget::default());

    let store = open(&path);
    assert_eq!(
        reconcile_format(store.as_ref(), &none).await.unwrap(),
        FormatOutcome::Recorded
    );
    BatchSink::new(store.clone(), Arc::new(Notify::new()))
        .deliver(accepted(&signed(5), SystemTime::now()))
        .await
        .unwrap();
    Batcher::new(
        store.clone(),
        batcher_config(Compression::None),
        Arc::new(Notify::new()),
    )
    .flush_all()
    .await
    .unwrap();

    // Prepared (uncompressed) batch outstanding: switching to zstd is refused.
    let err = reconcile_format(store.as_ref(), &zstd).await.unwrap_err();
    assert!(matches!(
        err,
        FormatError::Incompatible {
            ready_batches: 1,
            ..
        }
    ));

    // Once drained, the change is accepted.
    drain(anchor(&store, &target), &store).await;
    assert!(matches!(
        reconcile_format(store.as_ref(), &zstd).await.unwrap(),
        FormatOutcome::Updated { .. }
    ));
}

/// `POST` `body` to the gateway ingress, returning the HTTP status code.
async fn post_ingress(addr: std::net::SocketAddr, body: &[u8]) -> u16 {
    let mut stream = TcpStream::connect(addr).await.expect("connect ingress");
    let request = format!(
        "POST /v1/telemetry HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

fn gateway_config(dir: &Path, compression: Compression) -> Config {
    let key_file = dir.join("anchor.key");
    std::fs::write(&key_file, "//Alice\n").unwrap();

    let mut config = Config::default();
    config.http.listen = "127.0.0.1:0".parse().unwrap();
    config.auth.mode = AuthMode::None;
    config.pubsub.enabled = false;
    config.metrics.listen = "127.0.0.1:0".parse().unwrap();
    config.storage.path = Some(dir.join("batcher.redb"));
    config.batcher = batcher_config(compression);
    config.blockchain.enabled = true;
    // Nothing listens here: the anchor keeps retrying while ingestion works.
    config.blockchain.endpoint = Some("ws://127.0.0.1:1".to_string());
    config.blockchain.key_file = Some(key_file);
    config.blockchain.node_id = Some(1);
    config.blockchain.retry_min_secs = 1;
    config.blockchain.retry_max_secs = 1;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gateway_keeps_ingesting_with_chain_down_and_recovers_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let config = gateway_config(dir.path(), Compression::Zstd);

    // First run: accepted envelopes are persisted and, on graceful shutdown,
    // flushed into a durable ready batch even though the chain is unreachable.
    let app = EdgeApp::start(config.clone())
        .await
        .expect("gateway starts");
    let http_addr = app.http_addr.expect("http bound");
    for tag in 0..3u8 {
        assert_eq!(
            post_ingress(http_addr, &encode_envelope(&signed(tag))).await,
            202
        );
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    app.shutdown().await;

    let store = open(config.storage.path.as_ref().unwrap());
    let counts = store.counts().await.unwrap();
    assert_eq!(counts.pending_messages, 0);
    assert_eq!(counts.ready_batches, 1);
    let ready = store.ready_batches(usize::MAX).await.unwrap();
    assert_eq!(ready[0].message_count, 3);
    let payload = ready[0].payload.clone();
    drop(store);

    // Restart: the prepared batch is still there, byte for byte.
    let app = EdgeApp::start(config.clone())
        .await
        .expect("gateway restarts");
    app.shutdown().await;
    let store = open(config.storage.path.as_ref().unwrap());
    assert_eq!(
        store.ready_batches(usize::MAX).await.unwrap()[0].payload,
        payload
    );
    drop(store);

    // Changing the payload format while that batch is outstanding is refused.
    let mut changed = config.clone();
    changed.batcher.compression = Compression::None;
    let error = EdgeApp::start(changed)
        .await
        .err()
        .expect("startup refused");
    assert!(
        format!("{error:#}").contains("payload format changed"),
        "unexpected error: {error:#}"
    );
}
