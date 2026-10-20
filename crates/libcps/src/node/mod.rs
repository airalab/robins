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
//! Node handles: [`Node`] (latest finalized block) and [`NodeAt`] (pinned block).

mod watch;

pub use watch::{MetaUpdate, MetaWatcher, PayloadUpdate, PayloadWatcher};

use crate::{
    cps::{Cps, CpsAt},
    error::Result,
    read,
    transaction::{self, Transaction},
    types::{
        AccountId, Capability, CreateNode, Grant, IdentityAssignment, Meta, NodeId, NodeInfo,
        NodeSnapshot, Payload, ResolvedScope, ScopeId,
    },
};

/// Handle to a node addressed by its canonical [`NodeId`].
///
/// Created by [`Cps::node`]. Reads run immediately against the latest
/// finalized block (one block per call; use [`Cps::finalized`] for several
/// consistent reads). Mutations only build a [`Transaction`]: nothing is
/// submitted until [`Cps::submit`] is called.
///
/// Parent and children are returned as [`NodeId`]s. For the account-addressed
/// projection of the same tree see [`Address`](crate::Address).
#[derive(Clone, Copy)]
pub struct Node<'a> {
    cps: &'a Cps,
    id: NodeId,
}

impl<'a> Node<'a> {
    pub(crate) fn new(cps: &'a Cps, id: NodeId) -> Self {
        Self { cps, id }
    }

    /// Identifier of the node.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// Topology of the node, `None` if the node does not exist.
    pub async fn info(&self) -> Result<Option<NodeInfo>> {
        self.cps.finalized().await?.node(self.id).info().await
    }

    /// Parent of the node, `None` for a root node or a missing node.
    pub async fn parent(&self) -> Result<Option<NodeId>> {
        self.cps.finalized().await?.node(self.id).parent().await
    }

    /// Children of the node in creation order.
    pub async fn children(&self) -> Result<Vec<NodeId>> {
        self.cps.finalized().await?.node(self.id).children().await
    }

    /// Metadata of the node, `None` if unset.
    pub async fn meta(&self) -> Result<Option<Meta>> {
        self.cps.finalized().await?.node(self.id).meta().await
    }

    /// Payload of the node, `None` if unset.
    pub async fn payload(&self) -> Result<Option<Payload>> {
        self.cps.finalized().await?.node(self.id).payload().await
    }

    /// Scope the node resolves to, as computed by the runtime.
    ///
    /// # Errors
    ///
    /// [`Error::NodeNotFound`](crate::Error::NodeNotFound) if the node does not exist.
    pub async fn resolve_scope(&self) -> Result<ResolvedScope> {
        self.cps
            .finalized()
            .await?
            .node(self.id)
            .resolve_scope()
            .await
    }

    /// Whether `account` holds `capability` at the node, as decided by the runtime.
    pub async fn has_capability(
        &self,
        account: &AccountId,
        capability: Capability,
    ) -> Result<bool> {
        self.cps
            .finalized()
            .await?
            .node(self.id)
            .has_capability(account, capability)
            .await
    }

    /// Identity account bound to the node, `None` if it has none.
    pub async fn identity(&self) -> Result<Option<AccountId>> {
        self.cps.finalized().await?.node(self.id).identity().await
    }

    /// Complete state of the node at one block, `None` if the node does not exist.
    pub async fn snapshot(&self) -> Result<Option<NodeSnapshot>> {
        self.cps.finalized().await?.node(self.id).snapshot().await
    }

    /// Follow payload changes of the node, from the next block on.
    pub async fn watch_payload(&self) -> Result<PayloadWatcher> {
        PayloadWatcher::new(self.cps, self.id).await
    }

    /// Follow metadata changes of the node, from the next block on.
    pub async fn watch_meta(&self) -> Result<MetaWatcher> {
        MetaWatcher::new(self.cps, self.id).await
    }

    /// Set the metadata.
    ///
    /// # Errors
    ///
    /// [`Error::MetaTooLarge`](crate::Error::MetaTooLarge) above [`MAX_META_SIZE`](crate::MAX_META_SIZE).
    pub fn set_meta(&self, meta: impl AsRef<[u8]>) -> Result<Transaction<()>> {
        transaction::set_meta(self.id, Some(meta.as_ref().to_vec()))
    }

    /// Clear the metadata.
    pub fn clear_meta(&self) -> Transaction<()> {
        transaction::set_meta(self.id, None).expect("clearing never exceeds limits")
    }

    /// Set the payload.
    ///
    /// # Errors
    ///
    /// [`Error::PayloadTooLarge`](crate::Error::PayloadTooLarge) above [`MAX_PAYLOAD_SIZE`](crate::MAX_PAYLOAD_SIZE).
    pub fn set_payload(&self, payload: impl AsRef<[u8]>) -> Result<Transaction<()>> {
        transaction::set_payload(self.id, Some(payload.as_ref().to_vec()))
    }

    /// Clear the payload.
    pub fn clear_payload(&self) -> Transaction<()> {
        transaction::set_payload(self.id, None).expect("clearing never exceeds limits")
    }

    /// Create a child of this node.
    ///
    /// The transaction result is the [`NodeId`] of the new node. Use
    /// [`CreateNode::identity`] to bind an identity during creation.
    ///
    /// # Errors
    ///
    /// [`Error::MetaTooLarge`](crate::Error::MetaTooLarge) or
    /// [`Error::PayloadTooLarge`](crate::Error::PayloadTooLarge) for oversized data.
    pub fn create_child(&self, params: CreateNode) -> Result<Transaction<NodeId>> {
        transaction::create_node(Some(self.id), params.meta, params.payload, params.identity)
    }

    /// Delete the node.
    pub fn delete(&self) -> Transaction<()> {
        transaction::delete_node(self.id)
    }

    /// Make the node the root of a new scope owned by the signer.
    ///
    /// Submitting fails with [`Error::ScopeAlreadyExists`](crate::Error::ScopeAlreadyExists)
    /// if the node is already a scope root.
    pub fn create_scope(&self) -> Transaction<ScopeId> {
        transaction::create_scope(self.id)
    }

    /// Grant `account` the `grant` at the node.
    pub fn grant(&self, account: AccountId, grant: Grant) -> Result<Transaction<()>> {
        Ok(transaction::grant_access(self.id, account, grant))
    }

    /// Revoke `capability` of `account` at the node.
    pub fn revoke(&self, account: AccountId, capability: Capability) -> Result<Transaction<()>> {
        Ok(transaction::revoke_access(self.id, account, capability))
    }

    /// Bind `assignment` as the identity of the node, replacing the current one.
    pub fn set_identity(&self, assignment: IdentityAssignment) -> Result<Transaction<()>> {
        Ok(transaction::set_identity(self.id, assignment))
    }

    /// Remove the identity of the node.
    pub fn clear_identity(&self) -> Transaction<()> {
        transaction::clear_identity(self.id)
    }
}

/// Read-only handle to a node at a pinned block.
///
/// Created by [`CpsAt::node`]; every read uses the block of its [`CpsAt`].
#[derive(Clone, Copy)]
pub struct NodeAt<'a> {
    at: &'a CpsAt,
    id: NodeId,
}

impl<'a> NodeAt<'a> {
    pub(crate) fn new(at: &'a CpsAt, id: NodeId) -> Self {
        Self { at, id }
    }

    /// Identifier of the node.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// Topology of the node, `None` if the node does not exist.
    pub async fn info(&self) -> Result<Option<NodeInfo>> {
        read::info(self.at.state(), self.id).await
    }

    /// Parent of the node, `None` for a root node or a missing node.
    pub async fn parent(&self) -> Result<Option<NodeId>> {
        Ok(self.info().await?.and_then(|info| info.parent))
    }

    /// Children of the node in creation order.
    pub async fn children(&self) -> Result<Vec<NodeId>> {
        read::children(self.at.state(), self.id).await
    }

    /// Metadata of the node, `None` if unset.
    pub async fn meta(&self) -> Result<Option<Meta>> {
        read::meta(self.at.state(), self.id).await
    }

    /// Payload of the node, `None` if unset.
    pub async fn payload(&self) -> Result<Option<Payload>> {
        read::payload(self.at.state(), self.id).await
    }

    /// Scope the node resolves to, as computed by the runtime.
    pub async fn resolve_scope(&self) -> Result<ResolvedScope> {
        read::resolve_scope(self.at.state(), self.id).await
    }

    /// Whether `account` holds `capability` at the node, as decided by the runtime.
    pub async fn has_capability(
        &self,
        account: &AccountId,
        capability: Capability,
    ) -> Result<bool> {
        read::has_capability(self.at.state(), self.id, account, capability).await
    }

    /// Identity account bound to the node, `None` if it has none.
    pub async fn identity(&self) -> Result<Option<AccountId>> {
        read::identity_of(self.at.state(), self.id).await
    }

    /// Complete state of the node, `None` if the node does not exist.
    pub async fn snapshot(&self) -> Result<Option<NodeSnapshot>> {
        let Some(info) = self.info().await? else {
            return Ok(None);
        };
        Ok(Some(NodeSnapshot {
            id: self.id,
            parent: info.parent,
            scope: info.scope,
            identity: self.identity().await?,
            meta: self.meta().await?,
            payload: self.payload().await?,
        }))
    }
}

/// Create a root node, the root of a new scope owned by the signer.
///
/// The transaction result is the [`NodeId`] of the new node. A root can only
/// get the signer's own account as identity.
///
/// # Errors
///
/// [`Error::MetaTooLarge`](crate::Error::MetaTooLarge) or
/// [`Error::PayloadTooLarge`](crate::Error::PayloadTooLarge) for oversized data.
pub fn create_root(params: CreateNode) -> Result<Transaction<NodeId>> {
    transaction::create_node(None, params.meta, params.payload, params.identity)
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn writes_build_without_submitting() {
        fn check(cps: &Cps) {
            let node = cps.node(NodeId::from(7));
            assert!(node.set_meta(b"meta").is_ok());
            assert!(node.set_payload(b"payload").is_ok());
            let _: Transaction<()> = node.clear_meta();
            let _: Transaction<()> = node.clear_payload();
            let _: Transaction<()> = node.delete();
            let _: Transaction<ScopeId> = node.create_scope();
            let _: Transaction<NodeId> = node.create_child(CreateNode::new()).unwrap();
            let account = AccountId::from([1u8; 32]);
            let _: Transaction<()> = node.grant(account, Grant::write_subtree()).unwrap();
            let _: Transaction<()> = node.revoke(account, Capability::Write).unwrap();
            let _: Transaction<()> = node.set_identity(IdentityAssignment::new(account)).unwrap();
            let _: Transaction<()> = node.clear_identity();
        }
        let _ = check;
        let _: Transaction<NodeId> = create_root(CreateNode::default()).unwrap();
    }

    #[test]
    fn create_scope_is_guarded() {
        use crate::transaction::{self, Precondition};

        let id = NodeId::from(3);
        assert_eq!(
            transaction::create_scope(id).precondition,
            Some(Precondition::NotScopeRoot(id))
        );
    }

    #[test]
    fn create_node_builder_collects_parameters() {
        let account = AccountId::from([2u8; 32]);
        let params = CreateNode::new()
            .meta(b"m")
            .payload(b"p")
            .identity(IdentityAssignment::new(account));
        assert_eq!(params.meta.as_deref(), Some(&b"m"[..]));
        assert_eq!(params.payload.as_deref(), Some(&b"p"[..]));
        assert_eq!(
            params.identity,
            Some(IdentityAssignment {
                account,
                permit: None
            })
        );
    }

    #[test]
    fn limits_are_inclusive() {
        fn check(node: Node<'_>) {
            assert!(node.set_meta(vec![0; MAX_META_SIZE]).is_ok());
            assert!(node.set_payload(vec![0; MAX_PAYLOAD_SIZE]).is_ok());
        }
        let _ = check;
        assert!(
            crate::transaction::set_meta(NodeId::from(1), Some(vec![0; MAX_META_SIZE])).is_ok()
        );
        assert!(
            crate::transaction::set_payload(NodeId::from(1), Some(vec![0; MAX_PAYLOAD_SIZE]))
                .is_ok()
        );
    }

    #[test]
    fn oversized_data_is_rejected() {
        let id = NodeId::from(1);
        let err = crate::transaction::set_meta(id, Some(vec![0; MAX_META_SIZE + 1])).unwrap_err();
        assert!(matches!(
            err,
            Error::MetaTooLarge { actual, max } if actual == MAX_META_SIZE + 1 && max == MAX_META_SIZE
        ));
        let err =
            crate::transaction::set_payload(id, Some(vec![0; MAX_PAYLOAD_SIZE + 1])).unwrap_err();
        assert!(matches!(err, Error::PayloadTooLarge { .. }));

        let err = create_root(CreateNode::new().meta(vec![0; MAX_META_SIZE + 1])).unwrap_err();
        assert!(matches!(err, Error::MetaTooLarge { .. }));
        let err =
            create_root(CreateNode::new().payload(vec![0; MAX_PAYLOAD_SIZE + 1])).unwrap_err();
        assert!(matches!(err, Error::PayloadTooLarge { .. }));
    }

    #[test]
    fn identity_errors_are_recognized_in_dispatch_text() {
        #[derive(Debug, thiserror::Error)]
        #[error("Module error: Cps(IdentityAlreadyInUse)")]
        struct Dispatch;

        assert!(matches!(
            Error::transaction(Dispatch),
            Error::Identity(IdentityError::AlreadyInUse)
        ));
    }

    #[test]
    fn read_futures_are_send() {
        fn assert_send<T: Send>(_: &T) {}
        async fn reads(cps: &Cps, node: NodeId, scope: ScopeId, account: AccountId) {
            let handle = cps.node(node);
            let _ = handle.info().await;
            let _ = handle.meta().await;
            let _ = handle.payload().await;
            let _ = handle.children().await;
            let _ = handle.parent().await;
            let _ = handle.identity().await;
            let _ = handle.snapshot().await;
            let _ = handle.resolve_scope().await;
            let _ = handle.has_capability(&account, Capability::Write).await;
            let _ = cps.scope(scope).info().await;
            let address = cps.address(account);
            let _ = address.node_id().await;
            let _ = address.parent().await;
            let _ = address.children().await;
            let _ = address.payload().await;
            if let Ok(at) = cps.finalized().await {
                let _ = at.node(node).snapshot().await;
                let _ = at.address(account).meta().await;
            }
            if let Ok(mut watcher) = handle.watch_payload().await {
                let _ = watcher.next().await;
            }
            if let Ok(mut watcher) = handle.watch_meta().await {
                let _ = watcher.next().await;
            }
        }
        fn check(cps: &Cps) {
            let fut = reads(
                cps,
                NodeId::from(1),
                ScopeId::from(1),
                AccountId::from([0u8; 32]),
            );
            assert_send(&fut);
        }
        let _ = check;
    }
}
