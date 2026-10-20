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
//! # libcps - Robonomics Cyber-Physical Systems Library
//!
//! High-level Rust API for the Robonomics CPS 1.1 pallet.
//!
//! ## Model
//!
//! ```text
//! Cps                   connection, handles, transaction submission
//!   node(NodeId)        Node     canonical structural interface
//!   address(AccountId)  Address  identity-account projection of the same tree
//!   finalized() / at()  CpsAt    reads pinned to one block (NodeAt, AddressAt)
//!   submit(tx, signer)  submit a Transaction<T>
//! Transaction<T>        opaque prepared CPS transaction
//! crypto::Signer<S>     identity: signs transactions and encrypts (S = Sr25519 | Ed25519)
//! ```
//!
//! [`Cps`] is the only entry point. [`NodeId`] and [`ScopeId`] are plain values.
//! Handles read immediately; mutations only build a [`Transaction`] that is
//! submitted explicitly with a signer, which `Cps` never stores. There is one
//! CPS tree with two ways to address it: [`NodeId`] is canonical, an
//! [`AccountId`] is an identity projection, and converting between them is
//! explicit and fallible ([`Node::identity`], [`Address::node_id`]). Subxt is
//! an internal implementation detail.
//!
//! ## Quick start
//!
//! ```no_run
//! use libcps::{crypto::{Signer, Sr25519}, Cps, NodeId};
//!
//! # async fn run() -> libcps::Result<()> {
//! let keypair = Signer::<Sr25519>::from_suri("//Alice")?;
//!
//! // Any JSON-RPC endpoint: a node, a remote service or a local `light::LightNode`.
//! let cps = Cps::connect("ws://127.0.0.1:9944").await?;
//!
//! let node = cps.node(NodeId::from(42));
//! let current = node.payload().await?;
//!
//! let tx = node.set_payload(b"hello")?;
//! let receipt = cps.submit_finalized(&tx, &keypair).await?;
//! println!("finalized in {:?}", receipt.block_hash);
//! # let _ = current;
//! # Ok(())
//! # }
//! ```
//!
//! ## Creating nodes, with an identity
//!
//! ```no_run
//! # use libcps::{crypto::{Signer, Sr25519}, Cps, CreateNode, IdentityAssignment};
//! # async fn run(cps: Cps, keypair: Signer<Sr25519>) -> libcps::Result<()> {
//! let root = cps
//!     .submit_finalized(&libcps::create_root(CreateNode::new())?, &keypair)
//!     .await?
//!     .result;
//!
//! // Without identity.
//! let tx = cps.node(root).create_child(CreateNode::new().meta(b"sensor"))?;
//! let child = cps.submit_finalized(&tx, &keypair).await?.result;
//!
//! // With the signer's own account as identity (another account needs a permit).
//! let tx = cps.node(root).create_child(
//!     CreateNode::new().identity(IdentityAssignment::new(keypair.account_id())),
//! )?;
//! let device = cps.submit_finalized(&tx, &keypair).await?.result;
//! # let _ = (child, device);
//! # Ok(())
//! # }
//! ```
//!
//! ## Reading by node ID and by account
//!
//! ```no_run
//! # use libcps::{Cps, NodeId, AccountId};
//! # async fn run(cps: Cps, id: NodeId, account: AccountId) -> libcps::Result<()> {
//! // By NodeId: results are NodeIds.
//! let parent: Option<NodeId> = cps.node(id).parent().await?;
//! let children: Vec<NodeId> = cps.node(id).children().await?;
//!
//! // By identity account: results are accounts.
//! let parent: Option<AccountId> = cps.address(account).parent().await?;
//! let payload = cps.address(account).payload().await?;
//!
//! // Explicit, fallible conversion between the two spaces.
//! let identity: Option<AccountId> = cps.node(id).identity().await?;
//! let node_id: Option<NodeId> = cps.address(account).node_id().await?;
//!
//! // Several reads at one block.
//! let at = cps.finalized().await?;
//! let info = at.node(id).info().await?;
//! let scope = at.node(id).resolve_scope().await?;
//! let payload = at.address(account).payload().await?;
//! # let _ = (parent, children, identity, node_id, info, scope, payload);
//! # Ok(())
//! # }
//! ```
//!
//! ## Scopes, access and identity
//!
//! Ownership belongs to the *scope* a node resolves to, not to the node.
//!
//! ```no_run
//! # use libcps::{crypto::{Signer, Sr25519}, *};
//! # async fn run(cps: Cps, keypair: Signer<Sr25519>, node: NodeId, account: AccountId) -> Result<()> {
//! let node = cps.node(node);
//!
//! if !node.has_capability(&account, Capability::Write).await? {
//!     let tx = node.grant(account, Grant::write_subtree())?;
//!     cps.submit_finalized(&tx, &keypair).await?;
//! }
//!
//! // `create_scope` refuses a node that already roots a scope; `replace_scope`
//! // replaces it explicitly.
//! let scope = cps.submit_finalized(&node.replace_scope(), &keypair).await?.result;
//!
//! cps.submit_finalized(&node.clear_identity(), &keypair).await?;
//! # let _ = scope;
//! # Ok(())
//! # }
//! ```
//!
//! ## Missing node vs unset field
//!
//! ```no_run
//! # use libcps::{Cps, NodeId};
//! # async fn run(cps: Cps, id: NodeId) -> libcps::Result<()> {
//! match cps.node(id).snapshot().await? {
//!     None => { /* the node does not exist */ }
//!     Some(node) => { /* `node.meta` and `node.payload` may be `None` */ }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Encryption
//!
//! The [`crypto`] module encrypts payloads with AEAD ciphers using ECDH key
//! agreement and HKDF. A [`crypto::Signer`] is tagged with its scheme
//! ([`crypto::Sr25519`] or [`crypto::Ed25519`]); it signs transactions and
//! implements [`crypto::Cipher`] (the two purposes stay separate). The receiver's
//! [`crypto::PublicKey`] carries the same tag, so keys of another scheme cannot
//! be mixed in: that is a compile error.
//!
//! ```
//! use libcps::crypto::{Cipher, EncryptionAlgorithm, Signer, Sr25519};
//!
//! # fn example() -> anyhow::Result<()> {
//! let alice = Signer::<Sr25519>::from_suri("//Alice")?;
//! let bob = Signer::<Sr25519>::from_suri("//Bob")?;
//!
//! let message = alice.encrypt(
//!     b"secret",
//!     &bob.public_key(),
//!     EncryptionAlgorithm::XChaCha20Poly1305,
//! )?;
//! assert_eq!(bob.decrypt(&message, Some(&alice.public_key()))?, b"secret");
//! # Ok(())
//! # }
//! # example().unwrap();
//! ```
//!
//! ## Feature flags
//!
//! - **`light-client`**: the `libcps::light` module, an embedded smoldot light node that serves a
//!   local JSON-RPC endpoint for [`Cps`]. Without it `libcps` does not depend on smoldot.
//! - **`cli`** (default): builds the `cps` command-line binary; always includes `light-client`.
//!
//! ## Safety
//!
//! This crate uses `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod address;
mod backend;
mod cps;
pub mod crypto;
mod error;
pub mod identity;
#[cfg(feature = "light-client")]
pub mod light;
mod node;
pub mod permit;
mod read;
mod scope;
mod transaction;
mod types;

pub(crate) use robonomics_runtime_subxt_api::api;

pub use address::{Address, AddressAt};
pub use cps::{Cps, CpsAt, PendingTransaction, TxReceipt};
pub use error::{Error, IdentityError, Result};
pub use identity::{IdentityKey, IdentityScheme};
pub use node::{create_root, MetaUpdate, MetaWatcher, Node, NodeAt, PayloadUpdate, PayloadWatcher};
pub use permit::{IdentityPermitPayload, PermitArtifact};
pub use scope::{Scope, ScopeAt};
pub use transaction::Transaction;
pub use types::{
    AccountId, Capability, CreateNode, Grant, GrantMode, Hash, IdentityAssignment, IdentityPermit,
    Meta, NodeId, NodeInfo, NodeSnapshot, Payload, PermitSignature, ResolvedScope, ScopeId,
    ScopeInfo, MAX_META_SIZE, MAX_PAYLOAD_SIZE,
};
