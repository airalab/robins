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
//! Extension traits implemented for [`NodeId`](crate::NodeId).
//!
//! Reads take a [`Client`](crate::Client) and execute immediately. Mutations
//! only build a [`Transaction`](crate::Transaction); submitting it is an
//! explicit, separate step performed by the client.

mod access;
mod read;
mod scope;
mod tree;
mod watch;
mod write;

pub use access::NodeAccess;
pub use read::NodeRead;
pub use scope::NodeScope;
pub use tree::{create_root, NodeTree};
pub use watch::{MetaUpdate, MetaWatcher, PayloadUpdate, PayloadWatcher};
pub use write::NodeWrite;
