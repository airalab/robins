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
//! `scope` commands: resolve, create, replace and show.
//!
//! Scope data is expressed in `NodeId` space whatever form the target has:
//! a scope root need not have an identity.

use crate::cli::{
    args::ReadArgs,
    commands::nav::NavArgs,
    context::{confirm, Ctx},
    output::{fields, info},
    target::Target,
};
use anyhow::Result;
use clap::{Args, Subcommand};
use libcps::ScopeId;
use serde_json::json;

/// Subcommands of `scope`.
#[derive(Subcommand, Debug)]
pub enum ScopeCommand {
    /// Show the scope a node resolves to
    Resolve(NavArgs),
    /// Make a node the root of a new scope, refusing an existing scope root
    Create(CreateArgs),
    /// Replace the scope rooted at a node, destroying its access entries
    Replace(ReplaceArgs),
    /// Show a scope record by id
    Show(ShowArgs),
}

/// Arguments of `scope create`.
#[derive(Args, Debug)]
#[command(long_about = "Make TARGET the root of a new scope owned by the signer.

The command refuses a node that already roots a scope, but that is a
client-side check against the finalized state: another transaction may create
or replace the scope before this one is included, so it is NOT an atomic
create-if-absent. Use `scope replace` to replace a scope deliberately.")]
pub struct CreateArgs {
    /// Node id or identity account
    pub target: Target,
}

/// Arguments of `scope replace`.
#[derive(Args, Debug)]
#[command(long_about = "Replace the scope rooted at TARGET by a new one.

DESTRUCTIVE: the existing scope's access entries are removed. Asks for
confirmation unless --yes is given.

EXAMPLES:
    cps scope replace 42
    cps scope replace 42 --yes")]
pub struct ReplaceArgs {
    /// Node id or identity account
    pub target: Target,

    /// Do not ask for confirmation
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// Arguments of `scope show`.
#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Scope id
    pub scope: u64,

    #[command(flatten)]
    pub read: ReadArgs,
}

/// Run a `scope` subcommand.
pub async fn run(ctx: &Ctx, command: ScopeCommand) -> Result<()> {
    match command {
        ScopeCommand::Resolve(args) => resolve(ctx, args).await,
        ScopeCommand::Create(args) => create(ctx, args).await,
        ScopeCommand::Replace(args) => replace(ctx, args).await,
        ScopeCommand::Show(args) => show(ctx, args).await,
    }
}

async fn resolve(ctx: &Ctx, args: NavArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let id = args.target.node_id(&at).await?;
    let scope = at.node(id).resolve_scope().await?;
    ctx.out.result(
        || {
            fields(&[
                ("Scope", scope.id.to_string()),
                ("Root", scope.root.to_string()),
                ("Owner", scope.owner.to_string()),
            ])
        },
        json!({
            "node": id.0,
            "scope": scope.id.0,
            "root": scope.root.0,
            "owner": scope.owner.to_string(),
            "path": scope.path.iter().map(|n| n.0).collect::<Vec<_>>(),
        }),
    );
    Ok(())
}

async fn create(ctx: &Ctx, args: CreateArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let receipt = ctx
        .submit(&cps, &cps.node(id).create_scope(), &signer)
        .await?;
    let scope = receipt.result;
    ctx.done(
        &receipt,
        format!("Node {id} now roots scope {scope}"),
        json!({ "node": id.0, "scope": scope.0 }),
    );
    Ok(())
}

async fn replace(ctx: &Ctx, args: ReplaceArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let node = cps.node(id);

    let question = match node.info().await?.and_then(|info| info.scope) {
        Some(scope) => format!(
            "Node {id} already roots scope {scope}.\nReplacing it will remove the existing scope access entries.\nContinue?"
        ),
        None => format!("Node {id} roots no scope yet. Replace (create) one?"),
    };
    if !confirm(&question, args.yes)? {
        info("Replacement cancelled");
        return Ok(());
    }
    let receipt = ctx.submit(&cps, &node.replace_scope(), &signer).await?;
    let scope = receipt.result;
    ctx.done(
        &receipt,
        format!("Node {id} now roots scope {scope}"),
        json!({ "node": id.0, "scope": scope.0 }),
    );
    Ok(())
}

async fn show(ctx: &Ctx, args: ShowArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let id = ScopeId(args.scope);
    let record = at
        .scope(id)
        .info()
        .await?
        .ok_or_else(|| anyhow::anyhow!("scope {id} not found"))?;
    ctx.out.result(
        || {
            fields(&[
                ("Scope", id.to_string()),
                ("Owner", record.owner.to_string()),
                ("Access", record.access_count.to_string()),
            ])
        },
        json!({
            "scope": id.0,
            "owner": record.owner.to_string(),
            "access_count": record.access_count,
        }),
    );
    Ok(())
}
