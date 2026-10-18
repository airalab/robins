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
//! Lifecycle and synchronization state of the light node.

/// State of a [`crate::light::LightNode`], observable through
/// [`LightNode::status`](crate::light::LightNode::status).
///
/// A running node moves `Starting -> RpcReady -> Syncing -> Ready`. The RPC
/// endpoint answers requests from `RpcReady` on, but finalized-state queries
/// are only useful once the node is `Ready`. A node may go back from `Ready` to
/// `Syncing` if it falls behind. `Stopped` and `Failed` are terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LightStatus {
    /// The chains are being initialized and the RPC endpoint is not listening yet.
    Starting,
    /// The RPC endpoint is listening, but no synchronization progress is known yet.
    RpcReady,
    /// The node is synchronizing the parachain.
    Syncing {
        /// Highest block proven finalized so far.
        finalized: u64,
        /// Highest block advertised by connected peers.
        target: u64,
        /// Number of connected peers.
        peers: u32,
    },
    /// The node follows the chain; finalized-state queries can be answered.
    Ready,
    /// The node was shut down.
    Stopped,
    /// The node stopped because of an error.
    Failed(String),
}

impl LightStatus {
    /// `true` once the node is synchronized enough to answer finalized-state queries.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// `true` if the node no longer runs ([`Self::Stopped`] or [`Self::Failed`]).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed(_))
    }
}

impl std::fmt::Display for LightStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => f.write_str("starting"),
            Self::RpcReady => f.write_str("rpc-ready"),
            Self::Syncing {
                finalized,
                target,
                peers,
            } => write!(
                f,
                "syncing (finalized #{finalized}, target #{target}, {peers} peers)"
            ),
            Self::Ready => f.write_str("ready"),
            Self::Stopped => f.write_str("stopped"),
            Self::Failed(reason) => write!(f, "failed: {reason}"),
        }
    }
}
