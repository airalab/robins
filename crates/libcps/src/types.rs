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
//! Plain value types of the CPS 1.0 data model.
//!
//! Identifiers are lightweight `Copy` values: they hold no client, signer or
//! cached chain state. Operations on them live in extension traits, see
//! [`crate::prelude`].

use crate::api::runtime_types::pallet_robonomics_cps as rt;
use std::fmt;

/// Account identifier (32-byte public key, SS58 `Display`/`FromStr`).
pub use robonomics_runtime_subxt_api::AccountId32 as AccountId;

/// 32-byte block or extrinsic hash.
pub use subxt::utils::H256 as Hash;

/// Maximum size in bytes of node metadata, as enforced by CPS pallet 1.0.
///
/// Mirrors the pallet constant `MAX_META_SIZE`; it is not read from the chain, so
/// keep it in sync when the pallet limit changes.
pub const MAX_META_SIZE: usize = 1024;

/// Maximum size in bytes of a node payload, as enforced by CPS pallet 1.0.
///
/// Mirrors the pallet constant `MAX_PAYLOAD_SIZE`; it is not read from the chain,
/// so keep it in sync when the pallet limit changes.
pub const MAX_PAYLOAD_SIZE: usize = 8192;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $rt:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);

        impl From<u64> for $name {
            fn from(id: u64) -> Self {
                Self(id)
            }
        }

        impl From<$name> for u64 {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl From<$name> for rt::$rt {
            fn from(id: $name) -> Self {
                rt::$rt(id.0)
            }
        }

        impl From<rt::$rt> for $name {
            fn from(id: rt::$rt) -> Self {
                Self(id.0)
            }
        }
    };
}

id_type!(
    /// Identifier of a CPS node, the central resource identifier of CPS 1.0.
    ///
    /// Node operations are provided by the extension traits
    /// [`NodeRead`](crate::NodeRead), [`NodeTree`](crate::NodeTree),
    /// [`NodeWrite`](crate::NodeWrite), [`NodeScope`](crate::NodeScope) and
    /// [`NodeAccess`](crate::NodeAccess).
    NodeId,
    NodeId
);

id_type!(
    /// Identifier of a CPS scope (ownership and access boundary).
    ///
    /// Read operations are provided by [`ScopeRead`](crate::ScopeRead).
    ScopeId,
    ScopeId
);

/// Operation that can be granted to an account within a scope (see
/// [`NodeAccess`](crate::NodeAccess)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Create a nested scope at a node.
    CreateScope,
    /// Set node metadata and payload.
    Write,
}

impl From<Capability> for rt::Capability {
    fn from(c: Capability) -> Self {
        match c {
            Capability::CreateScope => rt::Capability::CreateScope,
            Capability::Write => rt::Capability::Write,
        }
    }
}

/// How far a grant reaches from the node it is made on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GrantMode {
    /// The granted node only.
    Node,
    /// The granted node and its descendants within the same scope.
    Subtree,
}

impl From<GrantMode> for rt::GrantMode {
    fn from(m: GrantMode) -> Self {
        match m {
            GrantMode::Node => rt::GrantMode::Node,
            GrantMode::Subtree => rt::GrantMode::Subtree,
        }
    }
}

/// A capability together with its reach, the unit of a grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Access {
    /// Granted capability.
    pub capability: Capability,
    /// Reach of the grant.
    pub mode: GrantMode,
}

impl Access {
    /// Grant `capability` with the given `mode`.
    pub fn new(capability: Capability, mode: GrantMode) -> Self {
        Self { capability, mode }
    }

    /// [`Capability::Write`] on a single node.
    pub fn write_node() -> Self {
        Self::new(Capability::Write, GrantMode::Node)
    }

    /// [`Capability::Write`] on a node and its descendants in the same scope.
    pub fn write_subtree() -> Self {
        Self::new(Capability::Write, GrantMode::Subtree)
    }

    /// [`Capability::CreateScope`] on a single node.
    pub fn create_scope_node() -> Self {
        Self::new(Capability::CreateScope, GrantMode::Node)
    }

    /// [`Capability::CreateScope`] on a node and its descendants in the same scope.
    pub fn create_scope_subtree() -> Self {
        Self::new(Capability::CreateScope, GrantMode::Subtree)
    }
}

/// Node metadata bytes.
pub type Meta = Vec<u8>;

/// Node payload bytes.
pub type Payload = Vec<u8>;

/// Topology of a node, read from chain storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeInfo {
    /// Parent node, `None` for a root node.
    pub parent: Option<NodeId>,
    /// Scope rooted at this node, if it is a scope root.
    pub scope: Option<ScopeId>,
}

/// Scope record, read from chain storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeInfo {
    /// Account owning the scope.
    pub owner: AccountId,
    /// Number of access entries (distinct node and account pairs) stored under
    /// the scope; the pallet caps it per scope.
    pub access_count: u32,
}

/// Snapshot of the scope a node resolves to, as reported by the runtime
/// (`CpsApi::resolve_scope`).
///
/// This is data, not an active object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedScope {
    /// Identifier of the scope.
    pub id: ScopeId,
    /// Root node of the scope.
    pub root: NodeId,
    /// Account owning the scope.
    pub owner: AccountId,
    /// Nodes from the queried node (first) to the scope root (last), both
    /// included.
    pub path: Vec<NodeId>,
}

/// Parameters of a node creation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreateNode {
    /// Initial metadata.
    pub meta: Option<Meta>,
    /// Initial payload.
    pub payload: Option<Payload>,
}

impl NodeId {
    /// Runtime representation, for storage keys and calls.
    pub(crate) fn rt(self) -> rt::NodeId {
        self.into()
    }
}

impl ScopeId {
    /// Runtime representation, for storage keys and calls.
    pub(crate) fn rt(self) -> rt::ScopeId {
        self.into()
    }
}
