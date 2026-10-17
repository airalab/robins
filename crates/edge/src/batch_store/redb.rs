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
//! [`BatchStore`] backed by an embedded [`redb`] database.
//!
//! The database file holds three tables:
//!
//! | table              | key                  | value                                          |
//! |--------------------|----------------------|------------------------------------------------|
//! | `pending_messages` | `u64` sequence       | `received_at_ms (u64 LE) ‖ envelope bytes`     |
//! | `ready_batches`    | `u64` batch id       | `created_at_ms (u64 LE) ‖ count (u32 LE) ‖ payload` |
//! | `store_metadata`   | `&str`               | counters and the payload format descriptor      |
//!
//! `redb` is synchronous, so every operation runs on the blocking thread pool.
//! Each mutation is a single write transaction that is durable on commit; a
//! transaction dropped without committing leaves the database untouched.

use super::model::{
    BatchId, FormatDescriptor, MessageId, PreparedBatch, StoreCounts, StoreError, StoreResult,
    StoredBatch, StoredMessage, Timestamp,
};
use super::BatchStore;
use async_trait::async_trait;
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::Path;
use std::sync::Arc;

const PENDING_MESSAGES: TableDefinition<u64, &[u8]> = TableDefinition::new("pending_messages");
const READY_BATCHES: TableDefinition<u64, &[u8]> = TableDefinition::new("ready_batches");
const STORE_METADATA: TableDefinition<&str, &[u8]> = TableDefinition::new("store_metadata");

const NEXT_SEQUENCE: &str = "next_sequence";
const NEXT_BATCH_ID: &str = "next_batch_id";
const PAYLOAD_SCHEMA: &str = "payload_schema";
const PAYLOAD_SCHEMA_VERSION: &str = "payload_schema_version";
const PAYLOAD_ENCODING: &str = "payload_encoding";
const PAYLOAD_COMPRESSION: &str = "payload_compression";

/// Size of the `u64` receive/creation timestamp prefix of stored values.
const TIMESTAMP_LEN: usize = 8;
/// Size of the timestamp plus `u32` message count prefix of a stored batch.
const BATCH_HEADER_LEN: usize = TIMESTAMP_LEN + 4;

/// Embedded, crash-safe [`BatchStore`] on a single `redb` file.
#[derive(Clone)]
pub struct RedbBatchStore {
    db: Arc<Database>,
}

impl RedbBatchStore {
    /// Open the database at `path`, creating it (and its tables) if needed.
    ///
    /// Blocking: call before the async runtime is busy or from
    /// [`tokio::task::spawn_blocking`].
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let db = Database::create(path.as_ref()).map_err(StoreError::database)?;
        let txn = db.begin_write().map_err(StoreError::database)?;
        // Opening a table in a write transaction creates it, so later read
        // transactions never observe a missing table.
        txn.open_table(PENDING_MESSAGES)
            .map_err(StoreError::database)?;
        txn.open_table(READY_BATCHES)
            .map_err(StoreError::database)?;
        txn.open_table(STORE_METADATA)
            .map_err(StoreError::database)?;
        txn.commit().map_err(StoreError::database)?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Run a blocking database closure on the blocking thread pool.
    async fn blocking<T, F>(&self, f: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Database) -> StoreResult<T> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| StoreError::Task(e.to_string()))?
    }
}

#[async_trait]
impl BatchStore for RedbBatchStore {
    async fn push_message(
        &self,
        received_at: Timestamp,
        envelope: Vec<u8>,
    ) -> StoreResult<MessageId> {
        self.blocking(move |db| {
            let txn = db.begin_write().map_err(StoreError::database)?;
            let sequence = {
                let mut meta = txn
                    .open_table(STORE_METADATA)
                    .map_err(StoreError::database)?;
                let sequence = next_counter(&mut meta, NEXT_SEQUENCE)?;
                let mut pending = txn
                    .open_table(PENDING_MESSAGES)
                    .map_err(StoreError::database)?;
                pending
                    .insert(sequence, encode_message(received_at, &envelope).as_slice())
                    .map_err(StoreError::database)?;
                sequence
            };
            txn.commit().map_err(StoreError::database)?;
            Ok(MessageId(sequence))
        })
        .await
    }

    async fn pending_messages(&self, limit: usize) -> StoreResult<Vec<StoredMessage>> {
        self.blocking(move |db| {
            let txn = db.begin_read().map_err(StoreError::database)?;
            let pending = txn
                .open_table(PENDING_MESSAGES)
                .map_err(StoreError::database)?;
            pending
                .iter()
                .map_err(StoreError::database)?
                .take(limit)
                .map(|entry| {
                    let (key, value) = entry.map_err(StoreError::database)?;
                    decode_message(key.value(), value.value())
                })
                .collect()
        })
        .await
    }

    async fn commit_batches(
        &self,
        consumed: &[MessageId],
        prepared: &[PreparedBatch],
    ) -> StoreResult<Vec<BatchId>> {
        let consumed = consumed.to_vec();
        let prepared = prepared.to_vec();
        self.blocking(move |db| {
            let txn = db.begin_write().map_err(StoreError::database)?;
            let created_at = Timestamp::now();
            let mut ids = Vec::with_capacity(prepared.len());
            {
                let mut pending = txn
                    .open_table(PENDING_MESSAGES)
                    .map_err(StoreError::database)?;
                for message in &consumed {
                    let removed = pending.remove(message.0).map_err(StoreError::database)?;
                    if removed.is_none() {
                        // Returning drops the transaction, which aborts it.
                        return Err(StoreError::NotPending(*message));
                    }
                }

                let mut meta = txn
                    .open_table(STORE_METADATA)
                    .map_err(StoreError::database)?;
                let mut ready = txn
                    .open_table(READY_BATCHES)
                    .map_err(StoreError::database)?;
                for batch in &prepared {
                    let id = next_counter(&mut meta, NEXT_BATCH_ID)?;
                    ready
                        .insert(id, encode_batch(created_at, batch).as_slice())
                        .map_err(StoreError::database)?;
                    ids.push(BatchId(id));
                }
            }
            txn.commit().map_err(StoreError::database)?;
            Ok(ids)
        })
        .await
    }

    async fn ready_batches(&self) -> StoreResult<Vec<StoredBatch>> {
        self.blocking(|db| {
            let txn = db.begin_read().map_err(StoreError::database)?;
            let ready = txn
                .open_table(READY_BATCHES)
                .map_err(StoreError::database)?;
            ready
                .iter()
                .map_err(StoreError::database)?
                .map(|entry| {
                    let (key, value) = entry.map_err(StoreError::database)?;
                    decode_batch(key.value(), value.value())
                })
                .collect()
        })
        .await
    }

    async fn confirm_batch(&self, id: BatchId) -> StoreResult<()> {
        self.blocking(move |db| {
            let txn = db.begin_write().map_err(StoreError::database)?;
            {
                let mut ready = txn
                    .open_table(READY_BATCHES)
                    .map_err(StoreError::database)?;
                // Confirming twice (e.g. after a crash mid-cleanup) is harmless.
                ready.remove(id.0).map_err(StoreError::database)?;
            }
            txn.commit().map_err(StoreError::database)?;
            Ok(())
        })
        .await
    }

    async fn counts(&self) -> StoreResult<StoreCounts> {
        self.blocking(|db| {
            let txn = db.begin_read().map_err(StoreError::database)?;
            let pending = txn
                .open_table(PENDING_MESSAGES)
                .map_err(StoreError::database)?;
            let ready = txn
                .open_table(READY_BATCHES)
                .map_err(StoreError::database)?;
            Ok(StoreCounts {
                pending_messages: pending.len().map_err(StoreError::database)?,
                ready_batches: ready.len().map_err(StoreError::database)?,
            })
        })
        .await
    }

    async fn format_descriptor(&self) -> StoreResult<Option<FormatDescriptor>> {
        self.blocking(|db| {
            let txn = db.begin_read().map_err(StoreError::database)?;
            let meta = txn
                .open_table(STORE_METADATA)
                .map_err(StoreError::database)?;
            let read_text = |key: &str| -> StoreResult<Option<String>> {
                meta.get(key)
                    .map_err(StoreError::database)?
                    .map(|v| {
                        String::from_utf8(v.value().to_vec()).map_err(|_| {
                            StoreError::Corrupt(format!("metadata key {key} is not UTF-8"))
                        })
                    })
                    .transpose()
            };
            let (Some(schema), Some(version), Some(encoding), Some(compression)) = (
                read_text(PAYLOAD_SCHEMA)?,
                read_text(PAYLOAD_SCHEMA_VERSION)?,
                read_text(PAYLOAD_ENCODING)?,
                read_text(PAYLOAD_COMPRESSION)?,
            ) else {
                return Ok(None);
            };
            let schema_version = version.parse().map_err(|_| {
                StoreError::Corrupt(format!("invalid {PAYLOAD_SCHEMA_VERSION}: {version}"))
            })?;
            Ok(Some(FormatDescriptor {
                schema,
                schema_version,
                encoding,
                compression,
            }))
        })
        .await
    }

    async fn set_format_descriptor(&self, descriptor: &FormatDescriptor) -> StoreResult<()> {
        let descriptor = descriptor.clone();
        self.blocking(move |db| {
            let txn = db.begin_write().map_err(StoreError::database)?;
            {
                let mut meta = txn
                    .open_table(STORE_METADATA)
                    .map_err(StoreError::database)?;
                let version = descriptor.schema_version.to_string();
                for (key, value) in [
                    (PAYLOAD_SCHEMA, descriptor.schema.as_str()),
                    (PAYLOAD_SCHEMA_VERSION, version.as_str()),
                    (PAYLOAD_ENCODING, descriptor.encoding.as_str()),
                    (PAYLOAD_COMPRESSION, descriptor.compression.as_str()),
                ] {
                    meta.insert(key, value.as_bytes())
                        .map_err(StoreError::database)?;
                }
            }
            txn.commit().map_err(StoreError::database)?;
            Ok(())
        })
        .await
    }
}

/// Return the current value of the counter `key` and persist its successor.
fn next_counter(meta: &mut redb::Table<'_, &str, &[u8]>, key: &str) -> StoreResult<u64> {
    let current = match meta.get(key).map_err(StoreError::database)? {
        Some(value) => {
            let bytes: [u8; 8] = value
                .value()
                .try_into()
                .map_err(|_| StoreError::Corrupt(format!("counter {key} is not 8 bytes")))?;
            u64::from_le_bytes(bytes)
        }
        None => 0,
    };
    meta.insert(key, (current + 1).to_le_bytes().as_slice())
        .map_err(StoreError::database)?;
    Ok(current)
}

fn encode_message(received_at: Timestamp, envelope: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(TIMESTAMP_LEN + envelope.len());
    out.extend_from_slice(&received_at.as_millis().to_le_bytes());
    out.extend_from_slice(envelope);
    out
}

fn decode_message(sequence: u64, value: &[u8]) -> StoreResult<StoredMessage> {
    if value.len() < TIMESTAMP_LEN {
        return Err(StoreError::Corrupt(format!(
            "pending message {sequence} is truncated"
        )));
    }
    let (timestamp, envelope) = value.split_at(TIMESTAMP_LEN);
    Ok(StoredMessage {
        sequence: MessageId(sequence),
        received_at: Timestamp::from_millis(u64::from_le_bytes(
            timestamp.try_into().expect("split at 8 bytes"),
        )),
        envelope: envelope.to_vec(),
    })
}

fn encode_batch(created_at: Timestamp, batch: &PreparedBatch) -> Vec<u8> {
    let mut out = Vec::with_capacity(BATCH_HEADER_LEN + batch.payload.len());
    out.extend_from_slice(&created_at.as_millis().to_le_bytes());
    out.extend_from_slice(&batch.message_count.to_le_bytes());
    out.extend_from_slice(&batch.payload);
    out
}

fn decode_batch(id: u64, value: &[u8]) -> StoreResult<StoredBatch> {
    if value.len() < BATCH_HEADER_LEN {
        return Err(StoreError::Corrupt(format!(
            "ready batch {id} is truncated"
        )));
    }
    let (header, payload) = value.split_at(BATCH_HEADER_LEN);
    let (created_at, count) = header.split_at(TIMESTAMP_LEN);
    Ok(StoredBatch {
        id: BatchId(id),
        created_at: Timestamp::from_millis(u64::from_le_bytes(
            created_at.try_into().expect("split at 8 bytes"),
        )),
        message_count: u32::from_le_bytes(count.try_into().expect("4 header bytes")),
        payload: payload.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, RedbBatchStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = RedbBatchStore::open(dir.path().join("batcher.redb")).expect("open");
        (dir, store)
    }

    fn batch(payload: &[u8], count: u32) -> PreparedBatch {
        PreparedBatch {
            message_count: count,
            payload: payload.to_vec(),
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

    #[tokio::test]
    async fn persists_and_orders_pending_messages() {
        let (_dir, store) = store();
        for i in 0..5u8 {
            let id = store
                .push_message(Timestamp::from_millis(1000 + u64::from(i)), vec![i; 3])
                .await
                .unwrap();
            assert_eq!(id, MessageId(u64::from(i)));
        }
        let pending = store.pending_messages(3).await.unwrap();
        assert_eq!(pending.len(), 3);
        for (i, message) in pending.iter().enumerate() {
            assert_eq!(message.sequence, MessageId(i as u64));
            assert_eq!(message.received_at, Timestamp::from_millis(1000 + i as u64));
            assert_eq!(message.envelope, vec![i as u8; 3]);
        }
        assert_eq!(store.counts().await.unwrap().pending_messages, 5);
    }

    #[tokio::test]
    async fn pending_to_ready_is_atomic_and_assigns_increasing_ids() {
        let (_dir, store) = store();
        let a = store.push_message(Timestamp::now(), vec![1]).await.unwrap();
        let b = store.push_message(Timestamp::now(), vec![2]).await.unwrap();
        let c = store.push_message(Timestamp::now(), vec![3]).await.unwrap();

        let ids = store
            .commit_batches(&[a, b], &[batch(b"first", 1), batch(b"second", 1)])
            .await
            .unwrap();
        assert_eq!(ids, vec![BatchId(0), BatchId(1)]);

        let ready = store.ready_batches().await.unwrap();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].payload, b"first");
        assert_eq!(ready[1].payload, b"second");
        assert_eq!(ready[1].message_count, 1);

        let pending = store.pending_messages(10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sequence, c);
    }

    #[tokio::test]
    async fn failed_commit_loses_nothing() {
        let (_dir, store) = store();
        let a = store.push_message(Timestamp::now(), vec![1]).await.unwrap();

        // The second consumed message does not exist: the whole commit aborts.
        let err = store
            .commit_batches(&[a, MessageId(99)], &[batch(b"x", 2)])
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::NotPending(MessageId(99))));

        let counts = store.counts().await.unwrap();
        assert_eq!(counts.pending_messages, 1);
        assert_eq!(counts.ready_batches, 0);
        assert_eq!(store.pending_messages(10).await.unwrap()[0].sequence, a);
    }

    #[tokio::test]
    async fn state_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batcher.redb");
        {
            let store = RedbBatchStore::open(&path).unwrap();
            let a = store
                .push_message(Timestamp::from_millis(42), vec![9])
                .await
                .unwrap();
            store
                .push_message(Timestamp::from_millis(43), vec![8])
                .await
                .unwrap();
            store
                .commit_batches(&[a], &[batch(b"payload", 1)])
                .await
                .unwrap();
            store
                .set_format_descriptor(&descriptor("zstd"))
                .await
                .unwrap();
        }

        let store = RedbBatchStore::open(&path).unwrap();
        let pending = store.pending_messages(10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].received_at, Timestamp::from_millis(43));
        let ready = store.ready_batches().await.unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].payload, b"payload");
        assert_eq!(
            store.format_descriptor().await.unwrap(),
            Some(descriptor("zstd"))
        );

        // Counters keep increasing across restarts.
        let next = store.push_message(Timestamp::now(), vec![7]).await.unwrap();
        assert_eq!(next, MessageId(2));
    }

    #[tokio::test]
    async fn confirmed_batch_cleanup_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batcher.redb");
        {
            let store = RedbBatchStore::open(&path).unwrap();
            let a = store.push_message(Timestamp::now(), vec![1]).await.unwrap();
            let ids = store
                .commit_batches(&[a], &[batch(b"payload", 1)])
                .await
                .unwrap();
            store.confirm_batch(ids[0]).await.unwrap();
            // Idempotent.
            store.confirm_batch(ids[0]).await.unwrap();
        }
        let store = RedbBatchStore::open(&path).unwrap();
        assert!(store.ready_batches().await.unwrap().is_empty());
        assert_eq!(store.counts().await.unwrap(), StoreCounts::default());
    }

    #[tokio::test]
    async fn format_descriptor_absent_until_set() {
        let (_dir, store) = store();
        assert_eq!(store.format_descriptor().await.unwrap(), None);
        store
            .set_format_descriptor(&descriptor("none"))
            .await
            .unwrap();
        store
            .set_format_descriptor(&descriptor("zstd"))
            .await
            .unwrap();
        assert_eq!(
            store.format_descriptor().await.unwrap(),
            Some(descriptor("zstd"))
        );
    }
}
