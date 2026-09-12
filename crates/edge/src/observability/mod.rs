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
//! Observability foundation: structured logging, metrics and health probes.
//!
//! The daemon wires these together at startup:
//!
//! 1. [`init_tracing`] installs a `tracing` subscriber (logs to `stderr`).
//! 2. [`metrics::install`] registers the global Prometheus recorder.
//! 3. [`Health`] tracks liveness/readiness for probes.
//! 4. [`server::serve`] exposes `/health`, `/ready` and `/metrics`.
//!
//! Instrumentation elsewhere uses the [`metrics`] facade with the canonical
//! names defined in [`metrics`], and the [`tracing`] macros for logs.

pub mod health;
pub mod metrics;
pub mod server;

pub use health::Health;

use tracing_subscriber::filter::EnvFilter;

/// Initialise the global `tracing` subscriber for the daemon.
///
/// `directives` is an [`EnvFilter`] specification (e.g. `"info"` or
/// `"edge=debug,info"`); the `EDGE_LOG` environment variable overrides it when
/// set. Logs are written to `stderr` so they never contaminate stdout data.
///
/// This is idempotent: a second call (or a competing subscriber, such as the
/// CLI's `env_logger`) is ignored rather than panicking.
pub fn init_tracing(directives: &str) {
    let filter = EnvFilter::try_from_env("EDGE_LOG")
        .or_else(|_| EnvFilter::try_new(directives))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
