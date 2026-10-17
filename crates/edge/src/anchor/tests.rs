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
use crate::batch_store::{PreparedBatch, RedbBatchStore, Timestamp};
use crate::shutdown::ShutdownController;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Meta,
    SetMeta(Vec<u8>),
    SetPayload(Vec<u8>),
}

#[derive(Default)]
struct MockState {
    meta: Option<Vec<u8>>,
    calls: Vec<Call>,
    payload_failures: VecDeque<AnchorError>,
}

/// A scripted CPS node recording every call.
#[derive(Default)]
struct MockTarget {
    state: Mutex<MockState>,
}

impl MockTarget {
    fn with_meta(meta: Option<Vec<u8>>) -> Arc<Self> {
        let target = Self::default();
        target.state.lock().unwrap().meta = meta;
        Arc::new(target)
    }

    fn fail_payloads(&self, errors: Vec<AnchorError>) {
        self.state.lock().unwrap().payload_failures = errors.into();
    }

    fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    fn payloads(&self) -> Vec<Vec<u8>> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::SetPayload(p) => Some(p),
                _ => None,
            })
            .collect()
    }
}

fn confirmation() -> Confirmation {
    Confirmation {
        extrinsic_hash: "0xabc".into(),
        block_hash: "0xdef".into(),
    }
}

#[async_trait]
impl CpsTarget for MockTarget {
    async fn meta(&self) -> Result<Option<Vec<u8>>, AnchorError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(Call::Meta);
        Ok(state.meta.clone())
    }

    async fn set_meta(&self, meta: &[u8]) -> Result<Confirmation, AnchorError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(Call::SetMeta(meta.to_vec()));
        state.meta = Some(meta.to_vec());
        Ok(confirmation())
    }

    async fn set_payload(&self, payload: &[u8]) -> Result<Confirmation, AnchorError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(Call::SetPayload(payload.to_vec()));
        match state.payload_failures.pop_front() {
            Some(error) => Err(error),
            None => Ok(confirmation()),
        }
    }
}

fn descriptor(compression: &str) -> FormatDescriptor {
    FormatDescriptor {
        schema: "SignedEnvelopeBatch".into(),
        schema_version: 1,
        encoding: "protobuf".into(),
        compression: compression.into(),
    }
}

fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        min: Duration::from_millis(1),
        max: Duration::from_millis(5),
    }
}

fn store() -> (tempfile::TempDir, Arc<RedbBatchStore>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RedbBatchStore::open(dir.path().join("batcher.redb")).expect("open");
    (dir, Arc::new(store))
}

fn anchor(store: &Arc<RedbBatchStore>, target: &Arc<MockTarget>, max_payload: usize) -> Anchor {
    Anchor::new(
        store.clone(),
        target.clone(),
        descriptor("zstd"),
        fast_retry(),
        max_payload,
    )
}

/// Persist ready batches directly, as the batcher would.
async fn seed(store: &RedbBatchStore, payloads: &[&[u8]]) {
    let mut consumed = Vec::new();
    for _ in payloads {
        consumed.push(store.push_message(Timestamp::now(), vec![0]).await.unwrap());
    }
    let prepared: Vec<_> = payloads
        .iter()
        .map(|p| PreparedBatch {
            message_count: 1,
            payload: p.to_vec(),
        })
        .collect();
    store.commit_batches(&consumed, &prepared).await.unwrap();
}

/// Run the anchor until no ready batch is left (or it stops on its own).
async fn run_until_drained(anchor: Anchor, store: &Arc<RedbBatchStore>) -> Result<(), AnchorError> {
    let controller = ShutdownController::new();
    let ready = Arc::new(Notify::new());
    let task = tokio::spawn(anchor.run(ready, controller.subscribe()));
    for _ in 0..200 {
        if task.is_finished() || store.counts().await.unwrap().ready_batches == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    controller.trigger();
    task.await.unwrap()
}

#[tokio::test]
async fn matching_metadata_triggers_no_update() {
    let (_dir, store) = store();
    let current = MetadataBuilder::build(&descriptor("zstd"));
    let target = MockTarget::with_meta(Some(current));

    let outcome = anchor(&store, &target, 8192)
        .reconcile_metadata()
        .await
        .unwrap();
    assert_eq!(outcome, MetadataOutcome::Unchanged);
    assert_eq!(target.calls(), vec![Call::Meta]);
}

#[tokio::test]
async fn different_or_missing_metadata_is_updated_and_awaited() {
    for current in [None, Some(MetadataBuilder::build(&descriptor("none")))] {
        let (_dir, store) = store();
        let target = MockTarget::with_meta(current);

        let outcome = anchor(&store, &target, 8192)
            .reconcile_metadata()
            .await
            .unwrap();
        assert_eq!(outcome, MetadataOutcome::Updated(confirmation()));
        assert_eq!(
            target.calls(),
            vec![
                Call::Meta,
                Call::SetMeta(MetadataBuilder::build(&descriptor("zstd")))
            ]
        );
    }
}

#[tokio::test]
async fn metadata_is_reconciled_before_any_payload_and_exact_bytes_are_submitted() {
    let (_dir, store) = store();
    seed(&store, &[b"first-payload", b"second-payload"]).await;
    let target = MockTarget::with_meta(None);

    run_until_drained(anchor(&store, &target, 8192), &store)
        .await
        .unwrap();

    let calls = target.calls();
    assert_eq!(calls[0], Call::Meta);
    assert!(matches!(calls[1], Call::SetMeta(_)));
    assert_eq!(
        target.payloads(),
        vec![b"first-payload".to_vec(), b"second-payload".to_vec()]
    );
    // Confirmed batches are cleaned up durably.
    assert_eq!(store.counts().await.unwrap().ready_batches, 0);
}

#[tokio::test]
async fn transient_failures_retry_the_same_bytes() {
    let (_dir, store) = store();
    seed(&store, &[b"payload"]).await;
    let target = MockTarget::with_meta(Some(MetadataBuilder::build(&descriptor("zstd"))));
    target.fail_payloads(vec![
        AnchorError::Transient("rpc unavailable".into()),
        AnchorError::Transient("timeout".into()),
    ]);

    run_until_drained(anchor(&store, &target, 8192), &store)
        .await
        .unwrap();

    assert_eq!(target.payloads(), vec![b"payload".to_vec(); 3]);
    assert_eq!(store.counts().await.unwrap().ready_batches, 0);
}

#[tokio::test]
async fn permanent_failure_stops_the_anchor_and_keeps_the_batch() {
    let (_dir, store) = store();
    seed(&store, &[b"payload"]).await;
    let target = MockTarget::with_meta(Some(MetadataBuilder::build(&descriptor("zstd"))));
    target.fail_payloads(vec![AnchorError::Permanent("access denied".into())]);

    let result = run_until_drained(anchor(&store, &target, 8192), &store).await;

    assert!(matches!(result, Err(AnchorError::Permanent(_))));
    assert_eq!(
        target.payloads().len(),
        1,
        "permanent errors are not retried"
    );
    let ready = store.ready_batches().await.unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].payload, b"payload");
}

#[tokio::test]
async fn oversized_batch_is_rejected_not_repaired() {
    let (_dir, store) = store();
    seed(&store, &[b"0123456789"]).await;
    let target = MockTarget::with_meta(Some(MetadataBuilder::build(&descriptor("zstd"))));

    let result = run_until_drained(anchor(&store, &target, 4), &store).await;

    assert!(matches!(result, Err(AnchorError::Permanent(_))));
    assert!(target.payloads().is_empty(), "nothing is submitted");
    assert_eq!(
        store.ready_batches().await.unwrap()[0].payload,
        b"0123456789"
    );
}

#[tokio::test]
async fn ready_batches_survive_a_restart_and_are_resubmitted_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batcher.redb");
    {
        let store = RedbBatchStore::open(&path).unwrap();
        seed(&store, &[b"prepared-before-crash"]).await;
    }

    // "Restart": reopen the database and start a fresh anchor.
    let store = Arc::new(RedbBatchStore::open(&path).unwrap());
    let target = MockTarget::with_meta(Some(MetadataBuilder::build(&descriptor("zstd"))));
    run_until_drained(anchor(&store, &target, 8192), &store)
        .await
        .unwrap();

    assert_eq!(target.payloads(), vec![b"prepared-before-crash".to_vec()]);
    assert_eq!(store.counts().await.unwrap().ready_batches, 0);
}

#[tokio::test]
async fn picks_up_batches_committed_while_running() {
    let (_dir, store) = store();
    let target = MockTarget::with_meta(Some(MetadataBuilder::build(&descriptor("zstd"))));
    let controller = ShutdownController::new();
    let ready = Arc::new(Notify::new());
    let task =
        tokio::spawn(anchor(&store, &target, 8192).run(ready.clone(), controller.subscribe()));

    seed(&store, &[b"late"]).await;
    ready.notify_one();
    for _ in 0..200 {
        if !target.payloads().is_empty() && store.counts().await.unwrap().ready_batches == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    controller.trigger();
    task.await.unwrap().unwrap();

    assert_eq!(target.payloads(), vec![b"late".to_vec()]);
}
