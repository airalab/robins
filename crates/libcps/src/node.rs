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
//! Node-oriented API for CPS pallet interactions.
//!
//! This module provides an object-oriented interface for managing CPS nodes
//! on the Robonomics blockchain.
//!
//! # Architecture
//!
//! The `Node` struct encapsulates a reference to a blockchain client and a
//! [`NodeId`], providing methods to interact with that specific node.
//!
//! # Scopes and access
//!
//! Ownership does not belong to a node: it belongs to the *Scope* the node
//! resolves to. A Scope is rooted at a node and owned by one account. Nodes
//! without their own Scope resolve to the Scope of their nearest ancestor that
//! has one. The owner of a Scope may grant a [`Capability`] to other accounts
//! with a [`GrantMode`].
//!
//! This library deliberately does **not** reimplement Scope resolution or
//! authorization. [`Node::resolve_scope`] and [`Node::has_capability`] call the
//! canonical `CpsApi` runtime API, and mutating calls are authorized by the
//! runtime only; a rejected transaction is reported as an error.
//!
//! # Block consistency
//!
//! [`Node::query_at`] reads topology, meta, payload, children and the resolved
//! Scope from the same block, so the returned [`NodeInfo`] never mixes state
//! from different blocks. [`Node::query`] resolves the latest finalized block
//! once and delegates to it.
//!
//! # Examples
//!
//! ```no_run
//! use libcps::blockchain::{Client, Config, BoundedVec};
//! use libcps::node::{Capability, GrantMode, Node};
//! use subxt::utils::AccountId32;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = Config {
//!         ws_url: "ws://localhost:9944".to_string(),
//!         suri: Some("//Alice".to_string()),
//!     };
//!     let client = Client::new(&config).await?;
//!
//!     // Create a root node; the creator owns its new Scope.
//!     let meta = BoundedVec("sensor".as_bytes().to_vec());
//!     let payload = BoundedVec("22.5C".as_bytes().to_vec());
//!     let node = Node::create(&client, None, Some(meta), Some(payload)).await?;
//!
//!     // The owner now lives in the resolved Scope, not on the node.
//!     let info = node.query().await?;
//!     println!("owner: {}", info.scope.owner);
//!
//!     // Allow another account to write to this node and its descendants.
//!     let bob: AccountId32 = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
//!         .parse()
//!         .unwrap();
//!     node.grant_access(bob, Capability::Write, GrantMode::Subtree)
//!         .await?;
//!     assert!(node.has_capability(bob, Capability::Write).await?);
//!
//!     Ok(())
//! }
//! ```

use crate::blockchain::{api, BoundedVec, Client, ExtrinsicEvents};
use anyhow::{anyhow, Result};
use log::{debug, trace};
use parity_scale_codec::{Decode, Encode};
use subxt::utils::{AccountId32, H256};

pub use api::cps::events::{
    AccessGranted, AccessRevoked, MetaSet, NodeCreated, NodeDeleted, PayloadSet, ScopeCreated,
    ScopeDeleted,
};
pub use api::runtime_types::pallet_robonomics_cps::{Capability, GrantMode, NodeId, ScopeId};

/// Scope a node resolves to, as reported by the `CpsApi` runtime API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeInfo {
    /// Identifier of the Scope.
    pub id: ScopeId,
    /// Root node of the Scope.
    pub root: NodeId,
    /// Account owning the Scope.
    pub owner: AccountId32,
}

/// Snapshot of a CPS node at a single block.
///
/// Ownership is not a node property: see [`NodeInfo::scope`].
#[derive(Debug, Clone)]
pub struct NodeInfo {
    /// Node identifier.
    pub id: NodeId,
    /// Parent node, `None` for a root node.
    pub parent: Option<NodeId>,
    /// Node metadata, if set.
    pub meta: Option<BoundedVec<u8>>,
    /// Node payload, if set.
    pub payload: Option<BoundedVec<u8>>,
    /// Direct children of the node.
    pub children: Vec<NodeId>,
    /// Scope the node resolves to; carries the owner.
    pub scope: ScopeInfo,
}

/// SCALE wire format of the `CpsApi::resolve_scope` result.
///
/// The published runtime metadata (V14) carries no runtime API descriptions,
/// so the runtime API is called by name and its result decoded here. Only the
/// encoding is mirrored; no Scope semantics live in this crate.
#[derive(Decode)]
struct ResolvedScope {
    id: ScopeId,
    root: NodeId,
    owner: AccountId32,
    // Path from the node to the Scope root; part of the wire format only.
    _path: BoundedVec<NodeId>,
}

impl From<ResolvedScope> for ScopeInfo {
    fn from(scope: ResolvedScope) -> Self {
        Self {
            id: scope.id,
            root: scope.root,
            owner: scope.owner,
        }
    }
}

/// Handle to a CPS node.
///
/// Holds a borrowed [`Client`] and a [`NodeId`]; creating one does not touch
/// the chain.
///
/// Nodes cannot be re-parented; there is no `move_to`:
///
/// ```compile_fail
/// # use libcps::node::Node;
/// # async fn reparent(node: Node<'_>) {
/// node.move_to(1).await.unwrap();
/// # }
/// ```
pub struct Node<'a> {
    client: &'a Client,
    id: NodeId,
}

impl<'a> Node<'a> {
    /// Create a handle to an existing node.
    ///
    /// The node is not checked for existence.
    pub fn new(client: &'a Client, node_id: NodeId) -> Self {
        Self {
            client,
            id: node_id,
        }
    }

    /// Node identifier.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// Sign, submit and wait for finalization of a CPS call.
    async fn submit<Call>(&self, call: &Call, name: &str) -> Result<ExtrinsicEvents>
    where
        Call: subxt::transactions::Payload,
    {
        let keypair = self.client.require_keypair()?;
        self.client
            .api
            .tx()
            .await
            .map_err(|e| anyhow!("Failed to prepare transaction client: {}", e))?
            .sign_and_submit_then_watch_default(call, keypair)
            .await
            .map_err(|e| anyhow!("Failed to submit {} transaction: {}", name, e))?
            .wait_for_finalized_success()
            .await
            .map_err(|e| anyhow!("Transaction failed: {}", e))
    }

    /// Hash of the latest finalized block.
    async fn finalized_block_hash(&self) -> Result<H256> {
        Ok(self
            .client
            .api
            .at_current_block()
            .await
            .map_err(|e| anyhow!("Failed to get latest finalized block: {}", e))?
            .block_hash())
    }

    /// Create a new node under `parent` (or a root node if `None`).
    ///
    /// A root node starts a new Scope owned by the signer. The new node's id is
    /// read from the `NodeCreated` event.
    ///
    /// Meta and payload have separate runtime size limits; oversized values are
    /// rejected by the runtime.
    pub async fn create(
        client: &'a Client,
        parent: Option<NodeId>,
        meta: Option<BoundedVec<u8>>,
        payload: Option<BoundedVec<u8>>,
    ) -> Result<Self> {
        debug!(
            "Creating new CPS node: parent={:?}, has_meta={}, has_payload={}",
            parent,
            meta.is_some(),
            payload.is_some()
        );

        // The id is unknown until the event is read, so the handle is built
        // with a placeholder and replaced below.
        let placeholder = Self {
            client,
            id: NodeId(0),
        };
        let create_call = api::tx().cps().create_node(parent, meta, payload);
        let events = placeholder.submit(&create_call, "create_node").await?;

        let node_created_event = events
            .find_first::<NodeCreated>()
            .ok_or_else(|| anyhow!("NodeCreated event not found in transaction events"))?
            .map_err(|e| anyhow!("Failed to find NodeCreated event: {}", e))?;

        let node_id = node_created_event.0;
        debug!("CPS node created successfully: id={}", node_id.0);

        Ok(Self {
            client,
            id: node_id,
        })
    }

    /// Query the node at the latest finalized block.
    ///
    /// Resolves the block once and delegates to [`Node::query_at`].
    pub async fn query(&self) -> Result<NodeInfo> {
        let block_hash = self.finalized_block_hash().await?;
        trace!("Querying node {} at block {:?}", self.id.0, block_hash);
        self.query_at(block_hash).await
    }

    /// Query the node at a specific block.
    ///
    /// Topology (`Nodes`), data (`Meta`, `Payload`), `Children` and the
    /// resolved Scope (runtime API) are all read from `block_hash`.
    pub async fn query_at(&self, block_hash: H256) -> Result<NodeInfo> {
        let client_at_block = self
            .client
            .api
            .at_block(block_hash)
            .await
            .map_err(|e| anyhow!("Failed to access block {:?}: {}", block_hash, e))?;
        let storage = client_at_block.storage();

        let topology = storage
            .try_fetch(api::storage().cps().nodes(), (self.id,))
            .await
            .map_err(|e| anyhow!("Failed to query node storage at block: {}", e))?
            .ok_or_else(|| anyhow!("Node {} not found", self.id.0))?
            .decode()
            .map_err(|e| anyhow!("Failed to decode node storage value: {}", e))?;

        let meta = storage
            .try_fetch(api::storage().cps().meta(), (self.id,))
            .await
            .map_err(|e| anyhow!("Failed to query node meta at block: {}", e))?
            .map(|v| v.decode())
            .transpose()
            .map_err(|e| anyhow!("Failed to decode node meta: {}", e))?;

        let payload = storage
            .try_fetch(api::storage().cps().payload(), (self.id,))
            .await
            .map_err(|e| anyhow!("Failed to query node payload at block: {}", e))?
            .map(|v| v.decode())
            .transpose()
            .map_err(|e| anyhow!("Failed to decode node payload: {}", e))?;

        let children = storage
            .try_fetch(api::storage().cps().children(), (self.id,))
            .await
            .map_err(|e| anyhow!("Failed to query children at block: {}", e))?
            .map(|v| v.decode())
            .transpose()
            .map_err(|e| anyhow!("Failed to decode children storage value: {}", e))?
            .map(|v: BoundedVec<NodeId>| v.0)
            .unwrap_or_default();

        let scope = self.resolve_scope_at(block_hash).await?;

        Ok(NodeInfo {
            id: self.id,
            parent: topology.parent,
            meta,
            payload,
            children,
            scope,
        })
    }

    /// Blocking variant of [`Node::query`], for use inside a tokio runtime.
    pub fn query_blocking(&self) -> Result<NodeInfo> {
        tokio::runtime::Handle::current().block_on(self.query())
    }

    /// Resolve the Scope this node belongs to at the latest finalized block.
    pub async fn resolve_scope(&self) -> Result<ScopeInfo> {
        let block_hash = self.finalized_block_hash().await?;
        self.resolve_scope_at(block_hash).await
    }

    /// Resolve the Scope this node belongs to at `block_hash`, using
    /// `CpsApi::resolve_scope`.
    pub async fn resolve_scope_at(&self, block_hash: H256) -> Result<ScopeInfo> {
        let client_at_block = self
            .client
            .api
            .at_block(block_hash)
            .await
            .map_err(|e| anyhow!("Failed to access block {:?}: {}", block_hash, e))?;

        let resolved: Option<ResolvedScope> =
            call_cps_api(&client_at_block, "resolve_scope", self.id.encode()).await?;
        resolved
            .map(ScopeInfo::from)
            .ok_or_else(|| anyhow!("No Scope could be resolved for node {}", self.id.0))
    }

    /// Whether `account` may use `capability` at this node, at the latest
    /// finalized block, according to `CpsApi::has_capability`.
    pub async fn has_capability(
        &self,
        account: AccountId32,
        capability: Capability,
    ) -> Result<bool> {
        let block_hash = self.finalized_block_hash().await?;
        self.has_capability_at(block_hash, account, capability)
            .await
    }

    /// Whether `account` may use `capability` at this node at `block_hash`,
    /// according to `CpsApi::has_capability`.
    pub async fn has_capability_at(
        &self,
        block_hash: H256,
        account: AccountId32,
        capability: Capability,
    ) -> Result<bool> {
        let client_at_block = self
            .client
            .api
            .at_block(block_hash)
            .await
            .map_err(|e| anyhow!("Failed to access block {:?}: {}", block_hash, e))?;

        call_cps_api(
            &client_at_block,
            "has_capability",
            (self.id, account, capability).encode(),
        )
        .await
    }

    /// Set or clear the node's metadata.
    pub async fn set_meta(&self, meta: Option<BoundedVec<u8>>) -> Result<ExtrinsicEvents> {
        debug!(
            "Setting metadata for node {}: has_data={}",
            self.id.0,
            meta.is_some()
        );
        let call = api::tx().cps().set_meta(self.id, meta);
        self.submit(&call, "set_meta").await
    }

    /// Set or clear the node's payload.
    pub async fn set_payload(&self, payload: Option<BoundedVec<u8>>) -> Result<ExtrinsicEvents> {
        debug!(
            "Setting payload for node {}: has_data={}",
            self.id.0,
            payload.is_some()
        );
        let call = api::tx().cps().set_payload(self.id, payload);
        self.submit(&call, "set_payload").await
    }

    /// Create a Scope at this node.
    ///
    /// On an ordinary node this creates a new nested Scope owned by the
    /// signer. On an existing Scope root it replaces the Scope with a fresh
    /// [`ScopeId`], which is also how Scope ownership is changed.
    pub async fn create_scope(&self) -> Result<ExtrinsicEvents> {
        let call = api::tx().cps().create_scope(self.id);
        self.submit(&call, "create_scope").await
    }

    /// Grant `capability` to `principal` at this node, in the Scope the node
    /// resolves to.
    pub async fn grant_access(
        &self,
        principal: AccountId32,
        capability: Capability,
        mode: GrantMode,
    ) -> Result<ExtrinsicEvents> {
        let call = api::tx()
            .cps()
            .grant_access(self.id, principal, capability, mode);
        self.submit(&call, "grant_access").await
    }

    /// Revoke `capability` from `principal` at this node.
    pub async fn revoke_access(
        &self,
        principal: AccountId32,
        capability: Capability,
    ) -> Result<ExtrinsicEvents> {
        let call = api::tx()
            .cps()
            .revoke_access(self.id, principal, capability);
        self.submit(&call, "revoke_access").await
    }

    /// Delete this node.
    ///
    /// The runtime only allows deleting a node without children; deleting a
    /// Scope root also deletes its Scope.
    pub async fn delete(self) -> Result<ExtrinsicEvents> {
        let call = api::tx().cps().delete_node(self.id);
        self.submit(&call, "delete_node").await
    }
}

/// Call `CpsApi_<method>` at the block of `client_at_block` and decode the
/// SCALE-encoded result.
async fn call_cps_api<R: Decode>(
    client_at_block: &subxt::client::OnlineClientAtBlock<crate::blockchain::RobonomicsConfig>,
    method: &str,
    args: Vec<u8>,
) -> Result<R> {
    let function = format!("CpsApi_{method}");
    let bytes = client_at_block
        .runtime_apis()
        .call_raw(&function, Some(&args))
        .await
        .map_err(|e| anyhow!("{} failed: {}", function, e))?;
    R::decode(&mut &bytes[..]).map_err(|e| anyhow!("Failed to decode {} result: {}", function, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_scope_wire_format_decodes_into_scope_info() {
        let owner = AccountId32([7u8; 32]);
        let encoded = (ScopeId(3), NodeId(5), owner, vec![NodeId(9), NodeId(5)]).encode();

        let resolved = ResolvedScope::decode(&mut &encoded[..]).unwrap();
        let info = ScopeInfo::from(resolved);

        assert_eq!(
            info,
            ScopeInfo {
                id: ScopeId(3),
                root: NodeId(5),
                owner,
            }
        );
    }

    #[test]
    fn optional_resolved_scope_none_decodes() {
        let resolved = Option::<ResolvedScope>::decode(&mut &[0u8][..]).unwrap();
        assert!(resolved.is_none());
    }
}
