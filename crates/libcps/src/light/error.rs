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
//! Errors of the embedded light node.

use crate::error::Source;

/// Result alias of the [`crate::light`] module.
pub type Result<T, E = LightError> = std::result::Result<T, E>;

/// Failure categories of the embedded light node.
///
/// Smoldot types are never part of this type: the underlying error, if any, is
/// available through [`std::error::Error::source`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LightError {
    /// The node configuration is invalid.
    #[error("invalid light node configuration: {0}")]
    Config(String),

    /// A chain specification could not be parsed.
    #[error("invalid chain specification: {0}")]
    InvalidChainSpec(#[source] Source),

    /// A chain could not be added to the light client.
    #[error("chain bootstrap failed: {0}")]
    Bootstrap(#[source] Source),

    /// The local RPC listener could not be bound.
    #[error("cannot listen on {addr}: {source}")]
    Bind {
        /// Requested listen address.
        addr: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The local RPC server failed while running.
    #[error("RPC server failed: {0}")]
    Server(#[source] Source),

    /// Background tasks did not stop cleanly.
    #[error("light node shutdown failed: {0}")]
    Shutdown(#[source] Source),
}
