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
//! Scope handles: [`Scope`] (latest finalized block) and [`ScopeAt`] (pinned block).

use crate::{
    cps::{Cps, CpsAt},
    error::Result,
    read,
    types::{ScopeId, ScopeInfo},
};

/// Handle to a scope (ownership and access boundary), created by [`Cps::scope`].
#[derive(Clone, Copy)]
pub struct Scope<'a> {
    cps: &'a Cps,
    id: ScopeId,
}

impl<'a> Scope<'a> {
    pub(crate) fn new(cps: &'a Cps, id: ScopeId) -> Self {
        Self { cps, id }
    }

    /// Identifier of the scope.
    pub fn id(&self) -> ScopeId {
        self.id
    }

    /// Scope record, `None` if the scope does not exist.
    pub async fn info(&self) -> Result<Option<ScopeInfo>> {
        self.cps.finalized().await?.scope(self.id).info().await
    }
}

/// Read-only handle to a scope at a pinned block, created by [`CpsAt::scope`].
#[derive(Clone, Copy)]
pub struct ScopeAt<'a> {
    at: &'a CpsAt,
    id: ScopeId,
}

impl<'a> ScopeAt<'a> {
    pub(crate) fn new(at: &'a CpsAt, id: ScopeId) -> Self {
        Self { at, id }
    }

    /// Identifier of the scope.
    pub fn id(&self) -> ScopeId {
        self.id
    }

    /// Scope record, `None` if the scope does not exist.
    pub async fn info(&self) -> Result<Option<ScopeInfo>> {
        read::scope_info(self.at.state(), self.id).await
    }
}
