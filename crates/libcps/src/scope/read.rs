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
//! Scope queries.

use crate::{
    api,
    client::Client,
    error::{Error, Result},
    types::{ScopeId, ScopeInfo},
};
use std::future::Future;

/// Scope queries, read at the latest finalized block.
pub trait ScopeRead {
    /// Scope record, `None` if the scope does not exist.
    fn info(&self, client: &Client) -> impl Future<Output = Result<Option<ScopeInfo>>> + Send;
}

impl ScopeRead for ScopeId {
    async fn info(&self, client: &Client) -> Result<Option<ScopeInfo>> {
        let at = client.at_finalized().await?;
        let info = at
            .storage()
            .try_fetch(api::storage().cps().scopes(), (self.rt(),))
            .await
            .map_err(Error::runtime)?;
        info.map(|v| v.decode().map_err(Error::runtime))
            .transpose()
            .map(|info| {
                info.map(|info| ScopeInfo {
                    owner: info.owner,
                    access_count: info.access_count,
                })
            })
    }
}
