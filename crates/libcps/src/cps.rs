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
    api,
    backend::{self, Api, At},
    crypto::{Scheme, Signer},
    error::{Error, Result},
    node::{Node, NodeAt},
    permit::IdentityPermitPayload,
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

    /// Genesis hash of the chain, which binds identity permits to it.
    pub fn genesis_hash(&self) -> Hash {
        self.client.genesis_hash()
    }

    /// Payload an identity account signs to issue a permit for `scope`, valid
    /// until the block `valid_until`.
    ///
    /// The payload is built from the chain context (genesis hash) and the
    /// runtime limits, so the signing bytes are exactly what the runtime
    /// verifies. Signing is separate: use [`IdentityPermitPayload::sign`] or
    /// sign [`IdentityPermitPayload::signing_bytes`] elsewhere.
    ///
    /// # Errors
    ///
    /// [`Error::Crypto`] if `valid_until` is already past or further ahead than
    /// the runtime allows (relative to the latest finalized block), because the
    /// permit would be unusable.
    pub async fn identity_permit_payload(
        &self,
        scope: ScopeId,
        valid_until: u32,
    ) -> Result<IdentityPermitPayload> {
        let at = self.finalized().await?;
        let now = at.block_number();
        let ttl = at.max_identity_permit_ttl()?;
        if valid_until < now {
            return Err(Error::Crypto(format!(
                "permit validity {valid_until} is before the current block {now}"
            )));
        }
        if u64::from(valid_until) > u64::from(now) + u64::from(ttl) {
            return Err(Error::Crypto(format!(
                "permit validity {valid_until} exceeds the runtime limit: at most {ttl} blocks after block {now}"
            )));
        }
        Ok(IdentityPermitPayload {
            genesis_hash: self.genesis_hash(),
            scope,
            valid_until,
        })
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

    /// Number of the block all reads use.
    pub fn block_number(&self) -> u32 {
        u32::try_from(self.at.block_number()).unwrap_or(u32::MAX)
    }

    /// Maximum number of blocks an identity permit may be valid ahead of the
    /// current block (`MaxIdentityPermitTtl` runtime constant).
    pub fn max_identity_permit_ttl(&self) -> Result<u32> {
        self.at
            .constants()
            .entry(api::constants().cps().max_identity_permit_ttl())
            .map_err(Error::runtime)
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
        Self::receipt(&self.tx, in_block).await
    }

    /// Wait until the transaction is included in a best block (finalized or not)
    /// and successfully executed.
    ///
    /// The returned [`TxReceipt::block_hash`] is the including block, which may
    /// still be reverted by a reorganization until it is finalized.
    ///
    /// # Errors
    ///
    /// As [`PendingTransaction::wait_finalized`].
    pub async fn wait_in_block(mut self) -> Result<TxReceipt<T>> {
        use subxt::{
            error::{TransactionProgressError, TransactionStatusError},
            transactions::TransactionStatus,
        };
        let in_block = loop {
            let status = self
                .progress
                .next()
                .await
                .ok_or_else(|| {
                    Error::transaction(
                        TransactionProgressError::UnexpectedEndOfTransactionStatusStream,
                    )
                })?
                .map_err(Error::transaction)?;
            match status {
                TransactionStatus::InBestBlock(block)
                | TransactionStatus::InFinalizedBlock(block) => {
                    break block;
                }
                TransactionStatus::Error { message } => {
                    return Err(Error::transaction(TransactionStatusError::Error(message)));
                }
                TransactionStatus::Invalid { message } => {
                    return Err(Error::transaction(TransactionStatusError::Invalid(message)));
                }
                TransactionStatus::Dropped { message } => {
                    return Err(Error::transaction(TransactionStatusError::Dropped(message)));
                }
                _ => {}
            }
        };
        Self::receipt(&self.tx, in_block).await
    }

    /// Receipt of a transaction included in `in_block`, once its events report success.
    async fn receipt(
        tx: &Transaction<T>,
        in_block: subxt::transactions::TransactionInBlock<
            robonomics_runtime_subxt_api::RobonomicsConfig,
            subxt::client::OnlineClientAtBlockImpl<robonomics_runtime_subxt_api::RobonomicsConfig>,
        >,
    ) -> Result<TxReceipt<T>> {
        let block_hash = in_block.block_hash();
        let extrinsic_hash = in_block.extrinsic_hash();
        let events = in_block.wait_for_success().await.map_err(Error::outcome)?;
        Ok(TxReceipt {
            extrinsic_hash,
            block_hash,
            result: tx.decode_result(&events)?,
        })
    }
}

/// Outcome of a successfully executed transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxReceipt<T> {
    /// Hash of the extrinsic.
    pub extrinsic_hash: Hash,
    /// Hash of the block that includes the extrinsic: finalized, unless the
    /// transaction was awaited with [`PendingTransaction::wait_in_block`].
    pub block_hash: Hash,
    /// Semantic result: the created [`NodeId`](crate::NodeId) /
    /// [`ScopeId`](crate::ScopeId), or `()`.
    pub result: T,
}
