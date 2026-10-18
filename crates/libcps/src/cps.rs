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
//! [`Cps`], the single entry point to CPS: connection, handles, reads and submission.

use crate::{
    address::{Address, AddressAt},
    backend::{self, Api, At},
    crypto::{Scheme, Signer},
    error::{Error, Result},
    node::{Node, NodeAt},
    scope::{Scope, ScopeAt},
    transaction::{Precondition, Progress, Transaction},
    types::{AccountId, Hash, NodeId, ScopeId},
};

/// Connection to a Robonomics chain over JSON-RPC.
///
/// The endpoint may be a full node, a remote RPC service or a local
/// `libcps::light::LightNode` (feature `light-client`); `Cps` does not care.
///
/// `Cps` is the only entry point to CPS. It hands out lightweight handles:
///
/// - [`Cps::node`] and [`Cps::address`]: the two addressing interfaces of the
///   same CPS tree, by [`NodeId`] (canonical) or by identity [`AccountId`];
/// - [`Cps::finalized`] and [`Cps::at`]: a [`CpsAt`] pinned to one block, for
///   consistent multi-read queries.
///
/// Handles read immediately, but build [`Transaction`]s without submitting
/// them; [`Cps::submit`] does that with an explicit signer. `Cps` never stores a
/// signer.
///
/// ```no_run
/// # async fn run(node_id: libcps::NodeId) -> libcps::Result<()> {
/// let cps = libcps::Cps::connect("ws://127.0.0.1:9944").await?;
/// let info = cps.node(node_id).info().await?;
/// # Ok(()) }
/// ```
pub struct Cps {
    client: Api,
}

impl Cps {
    /// Connect to the RPC endpoint at `url` (`ws://` or `wss://`).
    ///
    /// # Errors
    ///
    /// [`Error::Connection`] if the endpoint cannot be reached.
    pub async fn connect(url: &str) -> Result<Self> {
        Ok(Self {
            client: backend::connect(url).await?,
        })
    }

    /// Underlying Subxt client, for crate-internal use only.
    pub(crate) fn client(&self) -> &Api {
        &self.client
    }

    /// Handle to the node `id`, addressed structurally. Reads use the latest
    /// finalized block, one block per call.
    pub fn node(&self, id: NodeId) -> Node<'_> {
        Node::new(self, id)
    }

    /// Handle to the node bound to the identity `account`, addressed by account.
    ///
    /// The same tree as [`Cps::node`], projected onto identity accounts: results
    /// are accounts, never [`NodeId`]s.
    pub fn address(&self, account: AccountId) -> Address<'_> {
        Address::new(self, account)
    }

    /// Handle to the scope `id`.
    pub fn scope(&self, id: ScopeId) -> Scope<'_> {
        Scope::new(self, id)
    }

    /// Read context pinned to the latest finalized block.
    ///
    /// All reads made through the returned [`CpsAt`] observe that one block.
    ///
    /// # Errors
    ///
    /// [`Error::Runtime`] if the finalized block cannot be fetched.
    pub async fn finalized(&self) -> Result<CpsAt> {
        let at = self
            .client
            .at_current_block()
            .await
            .map_err(Error::runtime)?;
        Ok(CpsAt { at })
    }

    /// Read context pinned to the block with the given hash.
    ///
    /// # Errors
    ///
    /// [`Error::Runtime`] if the block cannot be fetched.
    pub async fn at(&self, block_hash: Hash) -> Result<CpsAt> {
        let at = self
            .client
            .at_block(block_hash)
            .await
            .map_err(Error::runtime)?;
        Ok(CpsAt { at })
    }

    /// Sign `tx` with `signer` and submit it, without waiting for inclusion.
    ///
    /// `signer` is a [`crypto::Signer`](crate::crypto::Signer) of the SR25519 or ED25519 scheme.
    ///
    /// Use [`PendingTransaction::wait_finalized`] to wait for finality.
    ///
    /// # Errors
    ///
    /// [`Error::ScopeAlreadyExists`] if the transaction is a
    /// [`Node::create_scope`] of a node that is already a scope root,
    /// [`Error::Transaction`] if the transaction cannot be signed or submitted.
    pub async fn submit<T, S: Scheme>(
        &self,
        tx: &Transaction<T>,
        signer: &Signer<S>,
    ) -> Result<PendingTransaction<T>> {
        self.check_precondition(tx).await?;
        let progress = tx.submit(&self.client, signer).await?;
        Ok(PendingTransaction {
            progress,
            tx: tx.clone(),
        })
    }

    /// Sign `tx` with `signer`, submit it and wait until it is finalized
    /// and successfully executed.
    ///
    /// # Errors
    ///
    /// As [`Cps::submit`] and [`PendingTransaction::wait_finalized`].
    pub async fn submit_finalized<T, S: Scheme>(
        &self,
        tx: &Transaction<T>,
        signer: &Signer<S>,
    ) -> Result<TxReceipt<T>> {
        self.submit(tx, signer).await?.wait_finalized().await
    }

    /// Verify the client-side intent of `tx` against the finalized state.
    async fn check_precondition<T>(&self, tx: &Transaction<T>) -> Result<()> {
        match tx.precondition {
            Some(Precondition::NotScopeRoot(id)) => {
                let info = self.node(id).info().await?.ok_or(Error::NodeNotFound(id))?;
                match info.scope {
                    Some(_) => Err(Error::ScopeAlreadyExists(id)),
                    None => Ok(()),
                }
            }
            None => Ok(()),
        }
    }
}

/// Read context pinned to one block.
///
/// Created by [`Cps::finalized`] or [`Cps::at`]. Every read made through it, or
/// through the handles it creates, uses the same block, so the results are
/// mutually consistent.
///
/// ```no_run
/// # async fn run(cps: libcps::Cps, id: libcps::NodeId) -> libcps::Result<()> {
/// let at = cps.finalized().await?;
/// let node = at.node(id);
/// let info = node.info().await?;
/// let payload = node.payload().await?;
/// let scope = node.resolve_scope().await?;
/// # Ok(()) }
/// ```
pub struct CpsAt {
    at: At,
}

impl CpsAt {
    /// Hash of the block all reads use.
    pub fn block_hash(&self) -> Hash {
        self.at.block_hash()
    }

    /// Handle to the node `id` at this block.
    pub fn node(&self, id: NodeId) -> NodeAt<'_> {
        NodeAt::new(self, id)
    }

    /// Handle to the node bound to the identity `account`, at this block.
    pub fn address(&self, account: AccountId) -> AddressAt<'_> {
        AddressAt::new(self, account)
    }

    /// Handle to the scope `id` at this block.
    pub fn scope(&self, id: ScopeId) -> ScopeAt<'_> {
        ScopeAt::new(self, id)
    }

    /// Chain state of the pinned block, for crate-internal use only.
    pub(crate) fn state(&self) -> &At {
        &self.at
    }
}

/// A submitted transaction that has not been awaited yet.
pub struct PendingTransaction<T> {
    progress: Progress,
    tx: Transaction<T>,
}

impl<T> PendingTransaction<T> {
    /// Hash of the submitted extrinsic.
    pub fn extrinsic_hash(&self) -> Hash {
        self.progress.extrinsic_hash()
    }

    /// Wait until the transaction is finalized and successfully executed.
    ///
    /// # Errors
    ///
    /// [`Error::AccessDenied`] if the runtime rejected the call for lack of
    /// authorization, [`Error::Dispatch`] for any other runtime dispatch failure,
    /// [`Error::Transaction`] for a submission or inclusion failure, and [`Error::Runtime`] if the created identifier cannot be
    /// decoded from the events.
    pub async fn wait_finalized(self) -> Result<TxReceipt<T>> {
        let in_block = self
            .progress
            .wait_for_finalized()
            .await
            .map_err(Error::transaction)?;
        let block_hash = in_block.block_hash();
        let extrinsic_hash = in_block.extrinsic_hash();
        let events = in_block.wait_for_success().await.map_err(Error::outcome)?;
        Ok(TxReceipt {
            extrinsic_hash,
            block_hash,
            result: self.tx.decode_result(&events)?,
        })
    }
}

/// Outcome of a finalized transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxReceipt<T> {
    /// Hash of the extrinsic.
    pub extrinsic_hash: Hash,
    /// Hash of the finalized block that includes the extrinsic.
    pub block_hash: Hash,
    /// Semantic result: the created [`NodeId`](crate::NodeId) /
    /// [`ScopeId`](crate::ScopeId), or `()`.
    pub result: T,
}
