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
//! Embedded Robonomics light client.
//!
//! [`LightNode`] runs a smoldot light client for the Polkadot relay chain and
//! the Robonomics parachain and serves it over a local WebSocket JSON-RPC
//! endpoint. [`crate::Cps`] stays RPC-only and connects to that endpoint like
//! to any other node:
//!
//! ```text
//!   Cps -> Subxt -> JSON-RPC --+-- full / remote node
//!                              |
//!                              +-- LightNode -> smoldot-light
//! ```
//!
//! Applications do not need to know about smoldot:
//!
//! ```no_run
//! use libcps::{light::{LightNode, LightNodeConfig}, Cps};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let light = LightNode::start(LightNodeConfig::default()).await?;
//! let cps = Cps::connect(light.rpc_url()).await?;
//! # drop(cps);
//! # Ok(())
//! # }
//! ```
//!
//! The endpoint binds to loopback by default and has no authentication. The
//! light node is stateless: it keeps no database, so every start synchronizes
//! from the chain specification checkpoints.

mod chain;
mod config;
mod error;
mod node;
mod server;
mod status;

pub use config::LightNodeConfig;
pub use error::{LightError, Result};
pub use node::LightNode;
pub use status::LightStatus;
