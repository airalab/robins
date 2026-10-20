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
//! Opaque prepared CPS transactions.

use crate::{
    api,
    backend::Api,
    crypto::{Scheme, Signer},
    error::{Error, Result},
    types::{
        AccountId, Capability, Grant, IdentityAssignment, NodeId, ScopeId, MAX_META_SIZE,
        MAX_PAYLOAD_SIZE,
    },
};
use robonomics_runtime_subxt_api::{BoundedVec, ExtrinsicEvents};
use std::{fmt, marker::PhantomData};

/// Decodes the semantic result of a transaction from its finalized events.
type Decoder<T> = fn(&ExtrinsicEvents) -> Result<T>;

/// CPS pallet call with already validated arguments.
#[derive(Clone, Debug)]
pub(crate) enum Call {
    CreateNode {
        parent: Option<NodeId>,
        meta: Option<Vec<u8>>,
        payload: Option<Vec<u8>>,
        identity: Option<IdentityAssignment>,
    },
    SetMeta(NodeId, Option<Vec<u8>>),
    SetPayload(NodeId, Option<Vec<u8>>),
    DeleteNode(NodeId),
    CreateScope(NodeId),
    GrantAccess(NodeId, AccountId, Grant),
    RevokeAccess(NodeId, AccountId, Capability),
    SetIdentity(NodeId, IdentityAssignment),
    ClearIdentity(NodeId),
}

/// Condition checked by [`Cps::submit`](crate::Cps::submit) before sending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Precondition {
    /// The node must not be a scope root yet.
    NotScopeRoot(NodeId),
}

/// Progress handle of a submitted transaction.
pub(crate) type Progress = subxt::transactions::TransactionProgress<
    robonomics_runtime_subxt_api::RobonomicsConfig,
    subxt::client::OnlineClientAtBlockImpl<robonomics_runtime_subxt_api::RobonomicsConfig>,
>;

/// A prepared, unsigned CPS transaction.
///
/// Building a transaction never touches the network. Submit it with
/// [`Cps::submit`](crate::Cps::submit) or
/// [`Cps::submit_finalized`](crate::Cps::submit_finalized). `T` is the
/// result available in the [`TxReceipt`](crate::TxReceipt) after success:
/// [`NodeId`] for node creation, [`ScopeId`] for scope creation, `()` otherwise.
pub struct Transaction<T> {
    pub(crate) call: Call,
    pub(crate) precondition: Option<Precondition>,
    decode: Decoder<T>,
    _result: PhantomData<fn() -> T>,
}

impl<T> Clone for Transaction<T> {
    fn clone(&self) -> Self {
        Self {
            call: self.call.clone(),
            precondition: self.precondition,
            decode: self.decode,
            _result: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Transaction<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transaction")
            .field("call", &self.call)
            .finish()
    }
}

impl<T> Transaction<T> {
    fn new(call: Call, decode: Decoder<T>) -> Self {
        Self {
            call,
            precondition: None,
            decode,
            _result: PhantomData,
        }
    }

    /// Decode the transaction result from the events of the finalized extrinsic.
    pub(crate) fn decode_result(&self, events: &ExtrinsicEvents) -> Result<T> {
        (self.decode)(events)
    }

    /// Sign with `signer` and submit, returning a progress stream.
    pub(crate) async fn submit<S: Scheme>(
        &self,
        api: &Api,
        signer: &Signer<S>,
    ) -> Result<Progress> {
        let mut tx = api.tx().await.map_err(Error::transaction)?;
        let cps = api::tx().cps();

        macro_rules! send {
            ($call:expr) => {
                tx.sign_and_submit_then_watch_default(&$call, signer)
                    .await
                    .map_err(Error::transaction)
            };
        }

        match self.call.clone() {
            Call::CreateNode {
                parent,
                meta,
                payload,
                identity,
            } => send!(cps.create_node(
                parent.map(Into::into),
                meta.map(BoundedVec),
                payload.map(BoundedVec),
                identity.map(|identity| identity.rt())
            )),
            Call::SetMeta(id, meta) => send!(cps.set_meta(id.into(), meta.map(BoundedVec))),
            Call::SetPayload(id, payload) => {
                send!(cps.set_payload(id.into(), payload.map(BoundedVec)))
            }
            Call::DeleteNode(id) => send!(cps.delete_node(id.into())),
            Call::CreateScope(id) => send!(cps.create_scope(id.into())),
            Call::GrantAccess(id, account, access) => send!(cps.grant_access(
                id.into(),
                account,
                access.capability.into(),
                access.mode.into()
            )),
            Call::RevokeAccess(id, account, capability) => {
                send!(cps.revoke_access(id.into(), account, capability.into()))
            }
            Call::SetIdentity(id, assignment) => {
                let permit = assignment.permit.map(|p| p.rt());
                send!(cps.set_identity(id.into(), assignment.account, permit))
            }
            Call::ClearIdentity(id) => send!(cps.clear_identity(id.into())),
        }
    }
}

fn no_result(_: &ExtrinsicEvents) -> Result<()> {
    Ok(())
}

fn created_node(events: &ExtrinsicEvents) -> Result<NodeId> {
    events
        .find_first::<api::cps::events::NodeCreated>()
        .ok_or_else(|| Error::runtime("NodeCreated event not found"))?
        .map(|event| event.0.into())
        .map_err(Error::runtime)
}

fn created_scope(events: &ExtrinsicEvents) -> Result<ScopeId> {
    events
        .find_first::<api::cps::events::ScopeCreated>()
        .ok_or_else(|| Error::runtime("ScopeCreated event not found"))?
        .map(|event| event.0.into())
        .map_err(Error::runtime)
}

fn check_meta(meta: &[u8]) -> Result<()> {
    if meta.len() > MAX_META_SIZE {
        return Err(Error::MetaTooLarge {
            actual: meta.len(),
            max: MAX_META_SIZE,
        });
    }
    Ok(())
}

fn check_payload(payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(Error::PayloadTooLarge {
            actual: payload.len(),
            max: MAX_PAYLOAD_SIZE,
        });
    }
    Ok(())
}

pub(crate) fn create_node(
    parent: Option<NodeId>,
    meta: Option<Vec<u8>>,
    payload: Option<Vec<u8>>,
    identity: Option<IdentityAssignment>,
) -> Result<Transaction<NodeId>> {
    if let Some(meta) = &meta {
        check_meta(meta)?;
    }
    if let Some(payload) = &payload {
        check_payload(payload)?;
    }
    Ok(Transaction::new(
        Call::CreateNode {
            parent,
            meta,
            payload,
            identity,
        },
        created_node,
    ))
}

pub(crate) fn set_meta(id: NodeId, meta: Option<Vec<u8>>) -> Result<Transaction<()>> {
    if let Some(meta) = &meta {
        check_meta(meta)?;
    }
    Ok(Transaction::new(Call::SetMeta(id, meta), no_result))
}

pub(crate) fn set_payload(id: NodeId, payload: Option<Vec<u8>>) -> Result<Transaction<()>> {
    if let Some(payload) = &payload {
        check_payload(payload)?;
    }
    Ok(Transaction::new(Call::SetPayload(id, payload), no_result))
}

pub(crate) fn delete_node(id: NodeId) -> Transaction<()> {
    Transaction::new(Call::DeleteNode(id), no_result)
}

/// Create a scope at a node that is not a scope root yet.
pub(crate) fn create_scope(id: NodeId) -> Transaction<ScopeId> {
    let mut tx = Transaction::new(Call::CreateScope(id), created_scope);
    tx.precondition = Some(Precondition::NotScopeRoot(id));
    tx
}

pub(crate) fn grant_access(id: NodeId, account: AccountId, access: Grant) -> Transaction<()> {
    Transaction::new(Call::GrantAccess(id, account, access), no_result)
}

pub(crate) fn revoke_access(
    id: NodeId,
    account: AccountId,
    capability: Capability,
) -> Transaction<()> {
    Transaction::new(Call::RevokeAccess(id, account, capability), no_result)
}

pub(crate) fn set_identity(id: NodeId, assignment: IdentityAssignment) -> Transaction<()> {
    Transaction::new(Call::SetIdentity(id, assignment), no_result)
}

pub(crate) fn clear_identity(id: NodeId) -> Transaction<()> {
    Transaction::new(Call::ClearIdentity(id), no_result)
}
