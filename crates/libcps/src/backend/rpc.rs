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
//! Direct RPC connection.

use super::Api;
use crate::error::{Error, Result};

/// Connect to a node RPC endpoint (`ws://`, `wss://`).
pub(crate) async fn connect_rpc(url: &str) -> Result<Api> {
    Api::from_url(url)
        .await
        .map_err(|e| Error::Connection(Box::new(e)))
}
