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
//! Network, query and transaction facade.

use crate::{
    backend::{self, Api, At},
    crypto::{Scheme, Signer},
    error::{Error, Result},
    transaction::{Progress, Transaction},
    types::Hash,
};
use parity_scale_codec::Decode;

/// Handle to a Robonomics node, over a direct RPC connection or an embedded
/// light client.
///
/// `Client` owns the connection and the transaction lifecycle; CPS operations
/// themselves are provided by extension traits on [`NodeId`](crate::NodeId) and
/// [`ScopeId`](crate::ScopeId). The RPC and light-client modes expose exactly
/// the same API.
pub struct Client {
    api: Api,
}

impl Client {
    /// Connect to the chain.
    ///
    /// - `Some(url)` connects to the RPC endpoint (`ws://` / `wss://`);
    /// - `None` starts an embedded light client syncing the Polkadot relay
    ///   chain and the Robonomics parachain (see [`Client::connect_light`]).
    ///
    /// # Errors
    ///
    /// [`Error::Connection`] for RPC endpoints, [`Error::LightClient`] when the
    /// light client cannot be started.
    pub async fn connect(url: Option<&str>) -> Result<Self> {
        match url {
            Some(url) => Self::connect_rpc(url).await,
            None => Self::connect_light().await,
        }
    }

    /// Connect to the RPC endpoint at `url`.
    pub async fn connect_rpc(url: &str) -> Result<Self> {
        Ok(Self {
            api: backend::connect_rpc(url).await?,
        })
    }

    /// Start the embedded light client.
    ///
    /// It uses the Polkadot relay chain and Robonomics parachain specs embedded in
    /// `robonomics-chain-spec`, so no endpoint is needed. The light client lives as long
    /// as the `Client`. The first queries may wait until the
    /// light client has synced a finalized block.
    pub async fn connect_light() -> Result<Self> {
        let api = backend::connect_light().await?;
        Ok(Self { api })
    }

    /// Underlying Subxt client, for crate-internal use only.
    pub(crate) fn api(&self) -> &Api {
        &self.api
    }

    /// Chain state at the latest finalized block.
    pub(crate) async fn at_finalized(&self) -> Result<At> {
        self.api.at_current_block().await.map_err(Error::runtime)
    }

    /// Hash of the latest finalized block.
    ///
    /// # Errors
    ///
    /// [`Error::Runtime`] if the finalized block cannot be fetched.
    pub async fn finalized_block_hash(&self) -> Result<Hash> {
        Ok(self.at_finalized().await?.block_hash())
    }

    /// Call the `CpsApi_<method>` runtime API and decode its SCALE result.
    pub(crate) async fn cps_api<R: Decode>(at: &At, method: &str, args: Vec<u8>) -> Result<R> {
        let function = format!("CpsApi_{method}");
        let bytes = at
            .runtime_apis()
            .call_raw(&function, Some(&args))
            .await
            .map_err(|e| Error::runtime(format!("{function} failed: {e}")))?;
        R::decode(&mut &bytes[..])
            .map_err(|e| Error::runtime(format!("failed to decode {function} result: {e}")))
    }

    /// Sign `tx` with `signer` and submit it, without waiting for inclusion.
    ///
    /// `signer` is a [`crypto::Signer`](crate::crypto::Signer) of the SR25519 or ED25519 scheme.
    ///
    /// Use [`PendingTransaction::wait_finalized`] to wait for finality.
    ///
    /// # Errors
    ///
    /// [`Error::Transaction`] if the transaction cannot be signed or submitted.
    pub async fn submit<T, S: Scheme>(
        &self,
        tx: &Transaction<T>,
        signer: &Signer<S>,
    ) -> Result<PendingTransaction<T>> {
        let progress = tx.submit(&self.api, signer).await?;
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
    /// As [`Client::submit`] and [`PendingTransaction::wait_finalized`].
    pub async fn submit_finalized<T, S: Scheme>(
        &self,
        tx: &Transaction<T>,
        signer: &Signer<S>,
    ) -> Result<TxReceipt<T>> {
        self.submit(tx, signer).await?.wait_finalized().await
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
    /// authorization, [`Error::Transaction`] for any other submission, inclusion or
    /// dispatch failure, and [`Error::Runtime`] if the created identifier cannot be
    /// decoded from the events.
    pub async fn wait_finalized(self) -> Result<TxReceipt<T>> {
        let in_block = self
            .progress
            .wait_for_finalized()
            .await
            .map_err(Error::transaction)?;
        let block_hash = in_block.block_hash();
        let extrinsic_hash = in_block.extrinsic_hash();
        let events = in_block
            .wait_for_success()
            .await
            .map_err(Error::transaction)?;
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
