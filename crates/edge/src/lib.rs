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
//! # edge - Robonomics Edge Gateway
//!
//! `edge` is a compact, single-binary "mini-connectivity" daemon for SBC/edge
//! devices. It accepts Connectivity Protocol [`SignedEnvelope`] messages over
//! multiple ingress transports, validates them through a single canonical
//! pipeline, deduplicates them, and publishes accepted messages to a native
//! libp2p GossipSub topic.
//!
//! ## Architecture
//!
//! The protocol is canonical and transports are adapters: every ingress produces
//! the same [`protocol::IngressMessage`], and authorization is always performed
//! against the envelope `sensor_id` (never transport metadata). Internal message
//! flow uses bounded channels to provide backpressure on constrained hardware.
//!
//! ## Modules
//!
//! - [`protocol`]: canonical envelope type, identity, validation and message-id.
//! - [`config`]: TOML + environment configuration.
//! - [`shutdown`]: coordinated graceful-shutdown primitive.
//! - [`cli`]: command-line interface (feature `cli`, enabled by default).
//!
//! [`SignedEnvelope`]: protocol::SignedEnvelope

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "cli")]
pub mod cli;
pub mod config;
pub mod protocol;
pub mod shutdown;
