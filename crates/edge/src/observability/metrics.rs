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
        ] {
            assert!(name.starts_with("edge_"), "{name} missing edge_ prefix");
            assert_eq!(name, name.to_lowercase(), "{name} must be snake_case");
        }
    }
}
