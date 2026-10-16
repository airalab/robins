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
//! Node metadata and payload mutations.

use crate::{
    error::Result,
    transaction::{self, Transaction},
    types::NodeId,
};

/// Mutations of node metadata and payload.
///
/// These only build transactions; nothing is submitted. Sizes are validated
/// up front and rejected with [`Error::MetaTooLarge`](crate::Error::MetaTooLarge)
/// / [`Error::PayloadTooLarge`](crate::Error::PayloadTooLarge). Authorization
/// ([`Capability::Write`](crate::Capability::Write) at the node) is checked by the
/// runtime when the transaction executes.
pub trait NodeWrite {
    /// Build a transaction setting the node metadata.
    ///
    /// Fails with [`Error::MetaTooLarge`](crate::Error::MetaTooLarge) if it exceeds [`MAX_META_SIZE`](crate::MAX_META_SIZE).
    fn set_meta(&self, meta: impl AsRef<[u8]>) -> Result<Transaction<()>>;

    /// Build a transaction clearing the node metadata.
    fn clear_meta(&self) -> Transaction<()>;

    /// Build a transaction setting the node payload.
    ///
    /// Fails with [`Error::PayloadTooLarge`](crate::Error::PayloadTooLarge) if it exceeds
    /// [`MAX_PAYLOAD_SIZE`](crate::MAX_PAYLOAD_SIZE).
    fn set_payload(&self, payload: impl AsRef<[u8]>) -> Result<Transaction<()>>;

    /// Build a transaction clearing the node payload.
    fn clear_payload(&self) -> Transaction<()>;
}

impl NodeWrite for NodeId {
    fn set_meta(&self, meta: impl AsRef<[u8]>) -> Result<Transaction<()>> {
        transaction::set_meta(*self, Some(meta.as_ref().to_vec()))
    }

    fn clear_meta(&self) -> Transaction<()> {
        transaction::set_meta(*self, None).expect("clearing never exceeds limits")
    }

    fn set_payload(&self, payload: impl AsRef<[u8]>) -> Result<Transaction<()>> {
        transaction::set_payload(*self, Some(payload.as_ref().to_vec()))
    }

    fn clear_payload(&self) -> Transaction<()> {
        transaction::set_payload(*self, None).expect("clearing never exceeds limits")
    }
}
