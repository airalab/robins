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
//! High-level Rust API for the Robonomics CPS 1.0 pallet.
//!
//! ## Model
//!
//! ```text
//! NodeId  + extension traits   node topology, data, scope and access
//! ScopeId + extension traits   scope queries
//! Client                       connection, queries, transaction submission
//! Transaction<T>               opaque prepared CPS transaction
//! crypto::Signer<S>            identity: signs transactions and encrypts (S = Sr25519 | Ed25519)
//! ```
//!
//! [`NodeId`] and [`ScopeId`] are plain values; they never hold a client,
//! signer or cached state. Reads take a [`Client`] and run immediately.
//! Mutations only build a [`Transaction`], which is submitted explicitly.
//! Subxt is an internal implementation detail.
//!
//! ## Quick start
//!
//! ```no_run
//! use libcps::{
//!     crypto::{Signer, Sr25519},
//!     prelude::*,
//!     Client, NodeId,
//! };
//!
//! # async fn run() -> libcps::Result<()> {
//! let keypair = Signer::<Sr25519>::from_suri("//Alice")?;
//!
//! // `Some(url)` uses RPC, `None` starts the embedded light client.
//! let client = Client::connect(Some("ws://127.0.0.1:9944")).await?;
//!
//! let node = NodeId::from(42);
//! let current = node.payload(&client).await?;
//!
//! let tx = node.set_payload(b"hello")?;
//! let receipt = client.submit_finalized(&tx, &keypair).await?;
//! println!("finalized in {:?}", receipt.block_hash);
//! # let _ = current;
//! # Ok(())
//! # }
//! ```
//!
//! ## Creating nodes
//!
//! ```no_run
//! # use libcps::{crypto::{Signer, Sr25519}, prelude::*, Client, CreateNode};
//! # async fn run(client: Client, keypair: Signer<Sr25519>) -> libcps::Result<()> {
//! let root = client
//!     .submit_finalized(&libcps::create_root(CreateNode::default())?, &keypair)
//!     .await?
//!     .result;
//!
//! let tx = root.create_child(CreateNode {
//!     meta: Some(b"sensor".to_vec()),
//!     payload: None,
//! })?;
//! let child = client.submit_finalized(&tx, &keypair).await?.result;
//! # let _ = child;
//! # Ok(())
//! # }
//! ```
//!
//! ## Scopes and access
//!
//! Ownership belongs to the *scope* a node resolves to, not to the node.
//!
//! ```no_run
//! # use libcps::{crypto::{Signer, Sr25519}, prelude::*, *};
//! # async fn run(client: Client, keypair: Signer<Sr25519>, node: NodeId, account: AccountId) -> Result<()> {
//! let resolved = node.resolve_scope(&client).await?;
//! let scope_info = resolved.id.info(&client).await?;
//!
//! if !node.has_capability(&client, &account, Capability::Write).await? {
//!     let tx = node.grant(account, Access::write_subtree())?;
//!     client.submit_finalized(&tx, &keypair).await?;
//! }
//! # let _ = scope_info;
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
//! - **`cli`** (default): builds the `cps` command-line binary.
//!
//! ## Safety
//!
//! This crate uses `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod backend;
mod client;
pub mod crypto;
mod error;
mod node;
pub mod prelude;
mod scope;
mod transaction;
mod types;

pub(crate) use robonomics_runtime_subxt_api::api;

pub use client::{Client, PendingTransaction, TxReceipt};
pub use error::{Error, Result};
pub use node::{
    create_root, MetaUpdate, MetaWatcher, NodeAccess, NodeRead, NodeScope, NodeTree, NodeWrite,
    PayloadUpdate, PayloadWatcher,
};
pub use scope::ScopeRead;
pub use transaction::Transaction;
pub use types::{
    Access, AccountId, Capability, CreateNode, GrantMode, Hash, Meta, NodeId, NodeInfo, Payload,
    ResolvedScope, ScopeId, ScopeInfo, MAX_META_SIZE, MAX_PAYLOAD_SIZE,
};
