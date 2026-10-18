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

use crate::types::{AccountId, NodeId};

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

    /// A storage query, runtime API call or SCALE decoding failed.
    #[error("runtime query failed: {0}")]
    Runtime(#[source] Source),

    /// A transaction could not be signed, submitted or finalized. Such failures
    /// are about delivery and may succeed when retried.
    #[error("transaction failed: {0}")]
    Transaction(#[source] Source),

    /// The transaction was finalized but the runtime rejected it with a
    /// dispatch error. Resubmitting the same call fails the same way and still
    /// costs fees, so this is permanent.
    #[error("transaction dispatch failed: {0}")]
    Dispatch(#[source] Source),

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

    /// Creating a scope was refused because the node is already a scope root;
    /// use `replace_scope` to replace it explicitly.
    #[error("node {0} is already a scope root, use replace_scope to replace it")]
    ScopeAlreadyExists(NodeId),

    /// The account is not the identity of any node.
    #[error("account {0} is not the identity of any node")]
    IdentityNotFound(AccountId),

    /// A node in the result has no identity, so it cannot be shown as an account.
    #[error("a node in the result has no identity")]
    MissingProjectedIdentity,

    /// The runtime rejected an identity operation.
    #[error("identity operation rejected: {0}")]
    Identity(IdentityError),

    /// A key could not be parsed or a cryptographic operation failed.
    #[error("crypto error: {0}")]
    Crypto(String),
}

/// Reason the runtime rejected an identity operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// The account is already the identity of another node.
    #[error("identity is already in use")]
    AlreadyInUse,
    /// The node has no identity.
    #[error("node has no identity")]
    NotSet,
    /// The identity is not the signer and no permit was given.
    #[error("permit required")]
    PermitRequired,
    /// The permit has expired.
    #[error("permit expired")]
    PermitExpired,
    /// The permit is valid for longer than the runtime allows.
    #[error("permit valid for too long")]
    PermitTooLong,
    /// The permit signature is invalid.
    #[error("invalid permit")]
    InvalidPermit,
    /// A root node can only get the signer as identity.
    #[error("root identity must be the signer")]
    RootIdentityMustBeCaller,
}

impl IdentityError {
    /// Recognize the pallet error named in a dispatch error text.
    fn classify(text: &str) -> Option<Self> {
        const NAMES: [(&str, IdentityError); 7] = [
            ("IdentityAlreadyInUse", IdentityError::AlreadyInUse),
            ("IdentityNotSet", IdentityError::NotSet),
            ("IdentityPermitRequired", IdentityError::PermitRequired),
            ("IdentityPermitExpired", IdentityError::PermitExpired),
            ("IdentityPermitTooLong", IdentityError::PermitTooLong),
            ("InvalidIdentityPermit", IdentityError::InvalidPermit),
            (
                "RootIdentityMustBeCaller",
                IdentityError::RootIdentityMustBeCaller,
            ),
        ];
        NAMES
            .iter()
            .find(|(name, _)| text.contains(name))
            .map(|(_, error)| *error)
    }
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
        } else if let Some(identity) = IdentityError::classify(&text) {
            Self::Identity(identity)
        } else {
            Self::Transaction(Box::new(e))
        }
    }

    /// Classify a failure while waiting for a transaction outcome.
    ///
    /// Runtime dispatch failures become [`Error::Dispatch`] (or the more
    /// specific authorization/identity errors); failing to fetch the block or
    /// its events stays a retryable [`Error::Transaction`].
    pub(crate) fn outcome(e: subxt::error::TransactionEventsError) -> Self {
        use subxt::error::TransactionEventsError as Events;
        let dispatched = matches!(
            e,
            Events::ExtrinsicFailed(_) | Events::CannotDecodeDispatchError { .. }
        );
        match Self::transaction(e) {
            Self::Transaction(source) if dispatched => Self::Dispatch(source),
            other => other,
        }
    }
}
