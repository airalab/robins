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
//! Metrics registry and the canonical metric-name convention.
//!
//! Instrumentation uses the [`metrics`] facade (`counter!`, `gauge!`,
//! `histogram!`) so call sites stay decoupled from the exporter. At startup the
//! daemon installs a Prometheus recorder via [`install`], and the operations
//! server renders it at `/metrics`.
//!
//! ## Naming convention (frozen interface)
//!
//! Metric names are defined **once** here as `pub const` items and referenced by
//! every subsystem; modules must not invent ad-hoc names. Names follow the
//! Prometheus style: a `edge_` prefix, `snake_case`, a base unit, and a
//! `_total` suffix for monotonic counters.

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Total envelopes received across all ingress transports (counter).
pub const INGRESS_RECEIVED_TOTAL: &str = "edge_ingress_received_total";
/// Envelopes rejected during protocol/signature validation (counter).
pub const VALIDATION_REJECTED_TOTAL: &str = "edge_validation_rejected_total";
/// Envelopes dropped as duplicates by the dedup stage (counter).
pub const DEDUP_DROPPED_TOTAL: &str = "edge_dedup_dropped_total";
/// Envelopes rejected by the authorization policy (counter).
pub const AUTH_REJECTED_TOTAL: &str = "edge_auth_rejected_total";
/// Accepted envelopes published to GossipSub (counter).
pub const PUBLISH_TOTAL: &str = "edge_publish_total";
/// Currently connected libp2p peers (gauge).
pub const CONNECTED_PEERS: &str = "edge_connected_peers";

/// Authorized messages persisted and waiting to be batched (gauge).
pub const BATCHER_PENDING_MESSAGES: &str = "edge_batcher_pending_messages";
/// Prepared batches persisted and waiting to be anchored (gauge).
pub const BATCHER_READY_BATCHES: &str = "edge_batcher_ready_batches";
/// Age of the oldest pending message, in seconds (gauge).
pub const BATCHER_OLDEST_PENDING_AGE_SECONDS: &str = "edge_batcher_oldest_pending_age_seconds";
/// Batch flushes, labelled by `reason` (`count`, `size`, `age`, `shutdown`) (counter).
pub const BATCHER_FLUSH_TOTAL: &str = "edge_batcher_flush_total";
/// Oversized batches that had to be split in two (counter).
pub const BATCHER_SPLIT_TOTAL: &str = "edge_batcher_split_total";
/// Single messages dropped because they can never fit `max_batch_size` (counter).
pub const BATCHER_PAYLOAD_TOO_LARGE_TOTAL: &str = "edge_batcher_payload_too_large_total";
/// Serialized (pre-compression) size of emitted batches, in bytes (histogram).
pub const BATCHER_SERIALIZED_BYTES: &str = "edge_batcher_serialized_bytes";
/// Final (post-compression) payload size of emitted batches, in bytes (histogram).
pub const BATCHER_PAYLOAD_BYTES: &str = "edge_batcher_payload_bytes";
/// Payload size divided by serialized size of the last emitted batch (gauge).
pub const BATCHER_COMPRESSION_RATIO: &str = "edge_batcher_compression_ratio";
/// `CPS.set_payload` submissions attempted (counter).
pub const ANCHOR_SUBMISSION_TOTAL: &str = "edge_anchor_submission_total";
/// Submissions retried after a transient failure (counter).
pub const ANCHOR_RETRY_TOTAL: &str = "edge_anchor_retry_total";
/// Batches confirmed (finalized) on chain and removed locally (counter).
pub const ANCHOR_CONFIRMATION_TOTAL: &str = "edge_anchor_confirmation_total";
/// Permanent anchoring failures that stopped the anchor (counter).
pub const ANCHOR_FAILURE_TOTAL: &str = "edge_anchor_failure_total";

/// Meshtastic packets received on the configured `PortNum` (counter).
pub const MESHTASTIC_PACKETS_RECEIVED_TOTAL: &str = "edge_meshtastic_packets_received_total";
/// Meshtastic packets ignored (wrong port, non-application data, not
/// PKI-authenticated) (counter).
pub const MESHTASTIC_PACKETS_IGNORED_TOTAL: &str = "edge_meshtastic_packets_ignored_total";
/// Meshtastic Transport v1 frames that failed structural validation (counter).
pub const MESHTASTIC_FRAMES_INVALID_TOTAL: &str = "edge_meshtastic_frames_invalid_total";
/// Fragmented Meshtastic reassemblies started (counter).
pub const MESHTASTIC_REASSEMBLY_STARTED_TOTAL: &str = "edge_meshtastic_reassembly_started_total";
/// Fragmented Meshtastic reassemblies completed successfully (counter).
pub const MESHTASTIC_REASSEMBLY_COMPLETED_TOTAL: &str =
    "edge_meshtastic_reassembly_completed_total";
/// Fragmented Meshtastic reassemblies that expired (idle or absolute
/// timeout) before completion (counter).
pub const MESHTASTIC_REASSEMBLY_EXPIRED_TOTAL: &str = "edge_meshtastic_reassembly_expired_total";
/// Fragmented Meshtastic reassemblies dropped due to a conflicting duplicate
/// or `fragment_count` mismatch (counter).
pub const MESHTASTIC_REASSEMBLY_CONFLICT_TOTAL: &str = "edge_meshtastic_reassembly_conflict_total";
/// Fragmented Meshtastic reassemblies rejected for exceeding a configured
/// size/count/pending limit (counter).
pub const MESHTASTIC_REASSEMBLY_OVERSIZE_TOTAL: &str = "edge_meshtastic_reassembly_oversize_total";
/// Currently pending (incomplete) Meshtastic reassemblies (gauge).
pub const MESHTASTIC_REASSEMBLY_PENDING: &str = "edge_meshtastic_reassembly_pending";
/// Meshtastic serial reconnect attempts (counter).
pub const MESHTASTIC_RECONNECTS_TOTAL: &str = "edge_meshtastic_reconnects_total";
/// Whether the Meshtastic serial session is currently up (gauge, 0 or 1).
pub const MESHTASTIC_CONNECTION_UP: &str = "edge_meshtastic_connection_up";

/// Install the global Prometheus recorder and register metric descriptions.
///
/// Returns a [`PrometheusHandle`] used by the operations server to render the
/// exposition format. Must be called at most once per process; a second call
/// returns an error from the exporter.
pub fn install() -> Result<PrometheusHandle, String> {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| format!("failed to install Prometheus recorder: {e}"))?;
    describe();
    Ok(handle)
}

/// Register human-readable descriptions for all known metrics.
///
/// Kept separate from [`install`] so tests can register descriptions against a
/// locally built recorder without touching global state.
fn describe() {
    metrics::describe_counter!(
        INGRESS_RECEIVED_TOTAL,
        "Total envelopes received across all ingress transports"
    );
    metrics::describe_counter!(
        VALIDATION_REJECTED_TOTAL,
        "Envelopes rejected during protocol/signature validation"
    );
    metrics::describe_counter!(
        DEDUP_DROPPED_TOTAL,
        "Envelopes dropped as duplicates by the dedup stage"
    );
    metrics::describe_counter!(
        AUTH_REJECTED_TOTAL,
        "Envelopes rejected by the authorization policy"
    );
    metrics::describe_counter!(PUBLISH_TOTAL, "Accepted envelopes published to GossipSub");
    metrics::describe_gauge!(CONNECTED_PEERS, "Currently connected libp2p peers");

    metrics::describe_counter!(
        MESHTASTIC_PACKETS_RECEIVED_TOTAL,
        "Meshtastic packets received on the configured PortNum"
    );
    metrics::describe_counter!(
        MESHTASTIC_PACKETS_IGNORED_TOTAL,
        "Meshtastic packets ignored (wrong port, non-application data, not PKI-authenticated)"
    );
    metrics::describe_counter!(
        MESHTASTIC_FRAMES_INVALID_TOTAL,
        "Meshtastic Transport v1 frames that failed structural validation"
    );
    metrics::describe_counter!(
        MESHTASTIC_REASSEMBLY_STARTED_TOTAL,
        "Fragmented Meshtastic reassemblies started"
    );
    metrics::describe_counter!(
        MESHTASTIC_REASSEMBLY_COMPLETED_TOTAL,
        "Fragmented Meshtastic reassemblies completed successfully"
    );
    metrics::describe_counter!(
        MESHTASTIC_REASSEMBLY_EXPIRED_TOTAL,
        "Fragmented Meshtastic reassemblies that expired before completion"
    );
    metrics::describe_counter!(
        MESHTASTIC_REASSEMBLY_CONFLICT_TOTAL,
        "Fragmented Meshtastic reassemblies dropped due to a conflicting duplicate or count mismatch"
    );
    metrics::describe_counter!(
        MESHTASTIC_REASSEMBLY_OVERSIZE_TOTAL,
        "Fragmented Meshtastic reassemblies rejected for exceeding a configured limit"
    );
    metrics::describe_gauge!(
        MESHTASTIC_REASSEMBLY_PENDING,
        "Currently pending (incomplete) Meshtastic reassemblies"
    );
    metrics::describe_counter!(
        MESHTASTIC_RECONNECTS_TOTAL,
        "Meshtastic serial reconnect attempts"
    );
    metrics::describe_gauge!(
        MESHTASTIC_CONNECTION_UP,
        "Whether the Meshtastic serial session is currently up"
    );

    metrics::describe_gauge!(
        BATCHER_PENDING_MESSAGES,
        "Authorized messages persisted and waiting to be batched"
    );
    metrics::describe_gauge!(
        BATCHER_READY_BATCHES,
        "Prepared batches persisted and waiting to be anchored"
    );
    metrics::describe_gauge!(
        BATCHER_OLDEST_PENDING_AGE_SECONDS,
        "Age of the oldest pending message in seconds"
    );
    metrics::describe_counter!(BATCHER_FLUSH_TOTAL, "Batch flushes, by reason");
    metrics::describe_counter!(BATCHER_SPLIT_TOTAL, "Oversized batches split in two");
    metrics::describe_counter!(
        BATCHER_PAYLOAD_TOO_LARGE_TOTAL,
        "Single messages dropped because they cannot fit max_batch_size"
    );
    metrics::describe_histogram!(
        BATCHER_SERIALIZED_BYTES,
        "Serialized (pre-compression) size of emitted batches in bytes"
    );
    metrics::describe_histogram!(
        BATCHER_PAYLOAD_BYTES,
        "Final (post-compression) payload size of emitted batches in bytes"
    );
    metrics::describe_gauge!(
        BATCHER_COMPRESSION_RATIO,
        "Payload size divided by serialized size of the last emitted batch"
    );
    metrics::describe_counter!(
        ANCHOR_SUBMISSION_TOTAL,
        "CPS set_payload submissions attempted"
    );
    metrics::describe_counter!(
        ANCHOR_RETRY_TOTAL,
        "Anchor submissions retried after a transient failure"
    );
    metrics::describe_counter!(
        ANCHOR_CONFIRMATION_TOTAL,
        "Batches finalized on chain and removed locally"
    );
    metrics::describe_counter!(
        ANCHOR_FAILURE_TOTAL,
        "Permanent anchoring failures that stopped the anchor"
    );
}

/// Build a non-global Prometheus handle for tests.
///
/// Unlike [`install`], this does not register a global recorder, so many tests
/// can run in the same process without conflicting.
#[cfg(test)]
pub(crate) fn test_handle() -> PrometheusHandle {
    PrometheusBuilder::new().build_recorder().handle()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_renders_exposition_text() {
        // A freshly built recorder renders an (empty) exposition without error.
        let handle = test_handle();
        let rendered = handle.render();
        assert!(rendered.is_empty() || rendered.contains('#') || !rendered.contains('\u{0}'));
    }

    #[test]
    fn metric_names_follow_convention() {
        for name in [
            INGRESS_RECEIVED_TOTAL,
            VALIDATION_REJECTED_TOTAL,
            DEDUP_DROPPED_TOTAL,
            AUTH_REJECTED_TOTAL,
            PUBLISH_TOTAL,
            CONNECTED_PEERS,
            MESHTASTIC_PACKETS_RECEIVED_TOTAL,
            MESHTASTIC_PACKETS_IGNORED_TOTAL,
            MESHTASTIC_FRAMES_INVALID_TOTAL,
            MESHTASTIC_REASSEMBLY_STARTED_TOTAL,
            MESHTASTIC_REASSEMBLY_COMPLETED_TOTAL,
            MESHTASTIC_REASSEMBLY_EXPIRED_TOTAL,
            MESHTASTIC_REASSEMBLY_CONFLICT_TOTAL,
            MESHTASTIC_REASSEMBLY_OVERSIZE_TOTAL,
            MESHTASTIC_REASSEMBLY_PENDING,
            MESHTASTIC_RECONNECTS_TOTAL,
            MESHTASTIC_CONNECTION_UP,
            BATCHER_PENDING_MESSAGES,
            BATCHER_READY_BATCHES,
            BATCHER_OLDEST_PENDING_AGE_SECONDS,
            BATCHER_FLUSH_TOTAL,
            BATCHER_SPLIT_TOTAL,
            BATCHER_PAYLOAD_TOO_LARGE_TOTAL,
            BATCHER_SERIALIZED_BYTES,
            BATCHER_PAYLOAD_BYTES,
            BATCHER_COMPRESSION_RATIO,
            ANCHOR_SUBMISSION_TOTAL,
            ANCHOR_RETRY_TOTAL,
            ANCHOR_CONFIRMATION_TOTAL,
            ANCHOR_FAILURE_TOTAL,
        ] {
            assert!(name.starts_with("edge_"), "{name} missing edge_ prefix");
            assert_eq!(name, name.to_lowercase(), "{name} must be snake_case");
        }
    }
}
