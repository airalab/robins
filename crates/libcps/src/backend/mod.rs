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
//! Private Subxt backend of the high-level API.
//!
//! Nothing in this module is part of the public API: Subxt types stay behind
//! [`crate::Client`] and [`crate::Transaction`].

mod light;
mod rpc;

pub(crate) use light::connect_light;
pub(crate) use rpc::connect_rpc;

use robonomics_runtime_subxt_api::RobonomicsConfig;

/// Subxt client used by the backend.
pub(crate) type Api = subxt::OnlineClient<RobonomicsConfig>;

/// Subxt view of the chain at one block.
pub(crate) type At = subxt::client::OnlineClientAtBlock<RobonomicsConfig>;
