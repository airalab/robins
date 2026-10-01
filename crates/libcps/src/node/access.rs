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
//! Node-relative authorization.

use crate::{
    client::Client,
    error::Result,
    transaction::{self, Transaction},
    types::{Access, AccountId, Capability, NodeId},
};
use parity_scale_codec::Encode;
use std::future::Future;

/// Authorization questions and mutations, relative to a node.
///
/// Grants apply from a specific node within the scope the node resolves to.
pub trait NodeAccess {
    /// Whether `account` may use `capability` at this node, as decided by the
    /// runtime (`CpsApi::has_capability`) at the latest finalized block.
    ///
    /// This applies the same checks as the pallet calls that require the
    /// capability (ownership of the node's scope, access entries, and for
    /// [`Capability::CreateScope`] ownership of the enclosing scope at a nested scope
    /// root), so prefer it over reading raw authorization storage. It returns
    /// `false` if the node does not exist.
    fn has_capability(
        &self,
        client: &Client,
        account: &AccountId,
        capability: Capability,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Build a transaction granting `access` to `account` at this node, within
    /// the scope the node resolves to.
    ///
    /// Only the owner of that scope may grant (checked by the runtime when the
    /// transaction executes). Granting a capability the account already holds here
    /// replaces its mode; the number of access entries per scope is limited.
    fn grant(&self, account: AccountId, access: Access) -> Result<Transaction<()>>;

    /// Build a transaction revoking `capability` from `account` at this node.
    ///
    /// Only the owner of the node's scope may revoke. It takes a [`Capability`]
    /// rather than an [`Access`]: the runtime revokes by capability, whatever its
    /// mode, and revoking a capability that was never granted succeeds.
    fn revoke(&self, account: AccountId, capability: Capability) -> Result<Transaction<()>>;
}

impl NodeAccess for NodeId {
    async fn has_capability(
        &self,
        client: &Client,
        account: &AccountId,
        capability: Capability,
    ) -> Result<bool> {
        let at = client.at_finalized().await?;
        let capability: crate::api::runtime_types::pallet_robonomics_cps::Capability =
            capability.into();
        Client::cps_api(
            &at,
            "has_capability",
            (self.rt(), *account, capability).encode(),
        )
        .await
    }

    fn grant(&self, account: AccountId, access: Access) -> Result<Transaction<()>> {
        Ok(transaction::grant_access(*self, account, access))
    }

    fn revoke(&self, account: AccountId, capability: Capability) -> Result<Transaction<()>> {
        Ok(transaction::revoke_access(*self, account, capability))
    }
}
