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
    backend::At,
    client::Client,
    error::{Error, Result},
    types::{Hash, ScopeId, ScopeInfo},
};
use std::future::Future;

/// Scope queries, read at the latest finalized block or, with `info_at`, at a given block.
pub trait ScopeRead {
    /// Scope record, `None` if the scope does not exist.
    fn info(&self, client: &Client) -> impl Future<Output = Result<Option<ScopeInfo>>> + Send;

    /// As [`info`](ScopeRead::info), at the block with hash `block_hash`.
    fn info_at(
        &self,
        client: &Client,
        block_hash: Hash,
    ) -> impl Future<Output = Result<Option<ScopeInfo>>> + Send;
}

impl ScopeRead for ScopeId {
    async fn info(&self, client: &Client) -> Result<Option<ScopeInfo>> {
        fetch_scope(*self, &client.at_finalized().await?).await
    }

    async fn info_at(&self, client: &Client, block_hash: Hash) -> Result<Option<ScopeInfo>> {
        fetch_scope(*self, &client.at_block(block_hash).await?).await
    }
}

/// Scope record of `scope` at the block `at`.
async fn fetch_scope(scope: ScopeId, at: &At) -> Result<Option<ScopeInfo>> {
    let info = at
        .storage()
        .try_fetch(api::storage().cps().scopes(), (scope.rt(),))
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
