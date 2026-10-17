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
use super::*;
use crate::batch_store::RedbBatchStore;
use crate::protocol::{
    decode_envelope_batch, encode_envelope, sign_message, SensorIdentity, SignOptions,
};
use crate::shutdown::ShutdownController;
use rand::Rng;

const MINUTE_MS: u64 = 60_000;

fn store() -> (tempfile::TempDir, Arc<RedbBatchStore>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RedbBatchStore::open(dir.path().join("batcher.redb")).expect("open");
    (dir, Arc::new(store))
}

fn config(compression: Compression) -> BatcherConfig {
    BatcherConfig {
        max_batch_age_secs: 30,
        max_batch_size: 8192,
        max_messages: 100,
        compression,
    }
}

fn batcher(store: &Arc<RedbBatchStore>, config: BatcherConfig) -> Batcher {
    Batcher::new(store.clone(), config, Arc::new(Notify::new()))
}

/// A signed envelope carrying `len` random (incompressible) bytes.
fn envelope(len: usize) -> SignedEnvelope {
    let mut message = vec![0u8; len];
    rand::rng().fill_bytes(&mut message);
    sign_message(
        &SensorIdentity::generate(),
        &message,
        SignOptions::default(),
    )
}

async fn push(store: &RedbBatchStore, envelope: &SignedEnvelope, received_at: Timestamp) {
    store
        .push_message(received_at, encode_envelope(envelope))
        .await
        .unwrap();
}

fn decoded(envelopes: &[SignedEnvelope]) -> Vec<Decoded> {
    envelopes
        .iter()
        .enumerate()
        .map(|(i, e)| Decoded {
            id: MessageId(i as u64),
            envelope: e.clone(),
        })
        .collect()
}

/// Decode the envelopes of a stored payload.
fn unpack(compression: Compression, payload: &[u8]) -> Vec<SignedEnvelope> {
    let serialized = compression.decompress(payload).unwrap();
    decode_envelope_batch(&serialized).unwrap().batch
}

#[tokio::test]
async fn flushes_when_max_messages_reached() {
    let (_dir, store) = store();
    let batcher = batcher(
        &store,
        BatcherConfig {
            max_messages: 3,
            ..config(Compression::None)
        },
    );
    let now = Timestamp::now();
    for _ in 0..3 {
        push(&store, &envelope(20), now).await;
    }

    let next = batcher.flush_due(now).await.unwrap();
    assert_eq!(next, None);
    let ready = store.ready_batches().await.unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].message_count, 3);
    assert_eq!(store.counts().await.unwrap().pending_messages, 0);
}

#[tokio::test]
async fn waits_until_age_deadline_from_original_receive_time() {
    let (_dir, store) = store();
    let batcher = batcher(&store, config(Compression::None));
    let now = Timestamp::from_millis(1_000_000);
    // Received 10 s before `now`, with a 30 s limit: 20 s left.
    push(&store, &envelope(20), Timestamp::from_millis(990_000)).await;

    let next = batcher.flush_due(now).await.unwrap();
    assert_eq!(next, Some(Duration::from_secs(20)));
    assert!(store.ready_batches().await.unwrap().is_empty());

    // Once the deadline passes the open batch is flushed.
    let later = Timestamp::from_millis(1_020_000);
    assert_eq!(batcher.flush_due(later).await.unwrap(), None);
    assert_eq!(store.ready_batches().await.unwrap().len(), 1);
}

#[tokio::test]
async fn restored_message_past_deadline_flushes_immediately() {
    let (_dir, store) = store();
    let batcher = batcher(&store, config(Compression::Zstd));
    let received = Timestamp::from_millis(Timestamp::now().as_millis() - 2 * MINUTE_MS);
    push(&store, &envelope(20), received).await;

    assert_eq!(batcher.flush_due(Timestamp::now()).await.unwrap(), None);
    assert_eq!(store.ready_batches().await.unwrap().len(), 1);
}

#[tokio::test]
async fn payload_is_exactly_the_serialized_batch_without_framing() {
    let (_dir, store) = store();
    let envelopes = [envelope(30), envelope(40)];
    let now = Timestamp::now();
    for e in &envelopes {
        push(&store, e, now).await;
    }

    batcher(&store, config(Compression::None))
        .flush_all()
        .await
        .unwrap();
    let ready = store.ready_batches().await.unwrap();
    let expected = protocol::encode_envelope_batch(&SignedEnvelopeBatch {
        batch: envelopes.to_vec(),
    });
    assert_eq!(ready[0].payload, expected);
}

#[tokio::test]
async fn zstd_payload_decompresses_to_the_batch() {
    let (_dir, store) = store();
    let envelopes = [envelope(30), envelope(40), envelope(50)];
    let now = Timestamp::now();
    for e in &envelopes {
        push(&store, e, now).await;
    }

    batcher(&store, config(Compression::Zstd))
        .flush_all()
        .await
        .unwrap();
    let ready = store.ready_batches().await.unwrap();
    assert_eq!(unpack(Compression::Zstd, &ready[0].payload), envelopes);
}

#[tokio::test]
async fn size_check_uses_final_payload_and_accepts_exact_fit() {
    let single = envelope(100);
    let batch = SignedEnvelopeBatch {
        batch: vec![single.clone()],
    };
    let exact = protocol::encode_envelope_batch(&batch).len();

    let (_dir, store) = store();
    let batcher = batcher(
        &store,
        BatcherConfig {
            max_batch_size: exact,
            ..config(Compression::None)
        },
    );
    push(&store, &single, Timestamp::now()).await;
    batcher.flush_all().await.unwrap();
    let ready = store.ready_batches().await.unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].payload.len(), exact);
}

#[tokio::test]
async fn single_oversized_message_is_a_permanent_error_and_dropped() {
    let single = envelope(100);
    let exact = protocol::encode_envelope_batch(&SignedEnvelopeBatch {
        batch: vec![single.clone()],
    })
    .len();

    let (_dir, store) = store();
    let batcher = batcher(
        &store,
        BatcherConfig {
            max_batch_size: exact - 1,
            ..config(Compression::None)
        },
    );

    let packed = batcher
        .pack(&decoded(std::slice::from_ref(&single)))
        .unwrap();
    assert!(packed.batches.is_empty());
    assert!(matches!(
        packed.rejected.as_slice(),
        [BatchError::PayloadTooLarge { max, payload, .. }] if *max == exact - 1 && *payload == exact
    ));

    // Through the store: the message is consumed rather than retried forever.
    push(&store, &single, Timestamp::now()).await;
    batcher.flush_all().await.unwrap();
    let counts = store.counts().await.unwrap();
    assert_eq!((counts.pending_messages, counts.ready_batches), (0, 0));
}

#[tokio::test]
async fn oversized_batch_is_split_without_loss_or_reordering() {
    let envelopes: Vec<_> = (0..12).map(|_| envelope(150)).collect();
    let (_dir, store) = store();
    let max = 700;
    let batcher = batcher(
        &store,
        BatcherConfig {
            max_batch_size: max,
            ..config(Compression::Zstd)
        },
    );

    let packed = batcher.pack(&decoded(&envelopes)).unwrap();
    assert!(
        packed.splits > 0,
        "12 x 150 B random payloads cannot fit 700 B"
    );
    assert!(packed.rejected.is_empty());

    let mut roundtrip = Vec::new();
    for (batch, _) in &packed.batches {
        assert!(batch.payload.len() <= max);
        let envelopes = unpack(Compression::Zstd, &batch.payload);
        assert_eq!(envelopes.len(), batch.message_count as usize);
        roundtrip.extend(envelopes);
    }
    assert_eq!(roundtrip, envelopes);
}

#[tokio::test]
async fn soft_size_flush_without_compression_never_needs_a_split() {
    let (_dir, store) = store();
    let batcher = batcher(
        &store,
        BatcherConfig {
            max_batch_size: 1000,
            ..config(Compression::None)
        },
    );
    let now = Timestamp::now();
    for _ in 0..3 {
        push(&store, &envelope(300), now).await;
    }

    // Two ~380 B envelopes fit in 1000 B; the third does not and stays open.
    let next = batcher.flush_due(now).await.unwrap();
    assert!(next.is_some());
    let ready = store.ready_batches().await.unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].message_count, 2);
    assert!(ready[0].payload.len() <= 1000);
    assert_eq!(store.counts().await.unwrap().pending_messages, 1);
}

#[tokio::test]
async fn flush_all_drains_everything_regardless_of_triggers() {
    let (_dir, store) = store();
    let batcher = batcher(&store, config(Compression::None));
    push(&store, &envelope(20), Timestamp::now()).await;

    batcher.flush_all().await.unwrap();
    let counts = store.counts().await.unwrap();
    assert_eq!((counts.pending_messages, counts.ready_batches), (0, 1));
}

#[tokio::test]
async fn ready_payload_is_unchanged_by_later_batching() {
    let (_dir, store) = store();
    let batcher = batcher(&store, config(Compression::Zstd));
    push(&store, &envelope(40), Timestamp::now()).await;
    batcher.flush_all().await.unwrap();
    let first = store.ready_batches().await.unwrap();

    push(&store, &envelope(40), Timestamp::now()).await;
    batcher.flush_all().await.unwrap();
    let all = store.ready_batches().await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0], first[0]);
}

#[tokio::test]
async fn run_loop_batches_on_wake_and_flushes_on_shutdown() {
    let (_dir, store) = store();
    let wake = Arc::new(Notify::new());
    let controller = ShutdownController::new();
    let batcher = Batcher::new(
        store.clone(),
        BatcherConfig {
            max_messages: 2,
            ..config(Compression::None)
        },
        wake.clone(),
    );
    let task = tokio::spawn(batcher.run(controller.subscribe()));

    // Two messages reach `max_messages` and are batched on wake-up.
    push(&store, &envelope(20), Timestamp::now()).await;
    push(&store, &envelope(20), Timestamp::now()).await;
    wake.notify_one();
    for _ in 0..100 {
        if store.counts().await.unwrap().ready_batches == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(store.counts().await.unwrap().ready_batches, 1);

    // A third, not-yet-due message is flushed when shutting down.
    push(&store, &envelope(20), Timestamp::now()).await;
    controller.trigger();
    task.await.unwrap();
    let counts = store.counts().await.unwrap();
    assert_eq!((counts.pending_messages, counts.ready_batches), (0, 2));
}
