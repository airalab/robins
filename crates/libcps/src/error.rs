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
//! High-level error type of `libcps`.

use crate::types::NodeId;

/// Boxed underlying error kept as a diagnostic source.
pub type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Result alias used across the `libcps` public API.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Failure categories of the `libcps` high-level API.
///
/// The underlying implementation error (if any) is available through
/// [`std::error::Error::source`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The connection to the chain could not be established.
    #[error("connection failed: {0}")]
    Connection(#[source] Source),

    /// An RPC request failed.
    #[error("RPC request failed: {0}")]
    Rpc(#[source] Source),

    /// The embedded light client could not be started.
    #[error("light client failed: {0}")]
    LightClient(#[source] Source),

    /// A storage query, runtime API call or SCALE decoding failed.
    #[error("runtime query failed: {0}")]
    Runtime(#[source] Source),

    /// A transaction could not be signed, submitted or finalized.
    #[error("transaction failed: {0}")]
    Transaction(#[source] Source),

    /// The node does not exist.
    #[error("node {0} not found")]
    NodeNotFound(NodeId),

    /// Node metadata exceeds the runtime limit.
    #[error("metadata is {actual} bytes, the maximum is {max}")]
    MetaTooLarge {
        /// Size of the supplied metadata in bytes.
        actual: usize,
        /// Maximum accepted size in bytes.
        max: usize,
    },

    /// Node payload exceeds the runtime limit.
    #[error("payload is {actual} bytes, the maximum is {max}")]
    PayloadTooLarge {
        /// Size of the supplied payload in bytes.
        actual: usize,
        /// Maximum accepted size in bytes.
        max: usize,
    },

    /// The runtime rejected the transaction because the signer is not
    /// authorized for the operation (`AccessDenied` or `NotScopeOwner` dispatch error).
    #[error("access denied")]
    AccessDenied,

    /// A key could not be parsed or a cryptographic operation failed.
    #[error("crypto error: {0}")]
    Crypto(String),
}

impl Error {
    pub(crate) fn runtime(e: impl Into<Source>) -> Self {
        Self::Runtime(e.into())
    }

    /// Classify a failed transaction, recognizing runtime authorization errors.
    pub(crate) fn transaction(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        let text = e.to_string();
        if text.contains("AccessDenied") || text.contains("NotScopeOwner") {
            Self::AccessDenied
        } else {
            Self::Transaction(Box::new(e))
        }
    }
}
