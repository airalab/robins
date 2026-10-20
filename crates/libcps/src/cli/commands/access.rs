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
//! `access` commands: check, grant and revoke capabilities in a scope.

use crate::cli::{
    args::{account_arg, ReadArgs},
    context::Ctx,
    target::Target,
};
use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use libcps::{AccountId, Capability, Grant, GrantMode};
use serde_json::json;
use std::process::ExitCode;

/// Capability that can be checked, granted and revoked.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CapabilityArg {
    /// Set node metadata and payload
    Write,
    /// Create a nested scope at a node
    CreateScope,
}

impl From<CapabilityArg> for Capability {
    fn from(capability: CapabilityArg) -> Self {
        match capability {
            CapabilityArg::Write => Capability::Write,
            CapabilityArg::CreateScope => Capability::CreateScope,
        }
    }
}

impl CapabilityArg {
    fn name(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::CreateScope => "create-scope",
        }
    }
}

/// How far a grant reaches.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum ModeArg {
    /// The target node only
    #[default]
    Node,
    /// The target node and its whole subtree
    Subtree,
}

impl From<ModeArg> for GrantMode {
    fn from(mode: ModeArg) -> Self {
        match mode {
            ModeArg::Node => GrantMode::Node,
            ModeArg::Subtree => GrantMode::Subtree,
        }
    }
}

/// Subcommands of `access`.
#[derive(Subcommand, Debug)]
pub enum AccessCommand {
    /// Check whether an account holds a capability at a node
    Check(CheckArgs),
    /// Grant a capability to an account
    Grant(GrantArgs),
    /// Revoke a capability from an account
    Revoke(RevokeArgs),
}

/// Arguments of `access check`.
#[derive(Args, Debug)]
#[command(
    long_about = "Check whether ACCOUNT holds CAPABILITY at TARGET, as decided by the runtime.

Prints `allowed` or `denied`. Exit status: 0 allowed, 1 denied, 2 on any CLI,
RPC or runtime error. --quiet prints nothing, for use in shell conditions.

EXAMPLES:
    cps access check 42 5FHneW... write
    cps access check 42 5FHneW... write --quiet && echo can write"
)]
pub struct CheckArgs {
    /// Node id or identity account
    pub target: Target,

    /// Account whose access is checked
    #[arg(value_parser = account_arg)]
    pub account: AccountId,

    /// Capability to check
    #[arg(value_enum)]
    pub capability: CapabilityArg,

    /// Print nothing; the exit status is the answer
    #[arg(short, long)]
    pub quiet: bool,

    #[command(flatten)]
    pub read: ReadArgs,
}

/// Arguments of `access grant`.
#[derive(Args, Debug)]
#[command(long_about = "Grant CAPABILITY to ACCOUNT at TARGET.

EXAMPLES:
    cps access grant 42 5FHneW... write
    cps access grant 42 5FHneW... write --mode subtree")]
pub struct GrantArgs {
    /// Node id or identity account
    pub target: Target,

    /// Account receiving the capability
    #[arg(value_parser = account_arg)]
    pub account: AccountId,

    /// Capability to grant
    #[arg(value_enum)]
    pub capability: CapabilityArg,

    /// Reach of the grant
    #[arg(long, value_enum, default_value_t = ModeArg::Node)]
    pub mode: ModeArg,
}

/// Arguments of `access revoke`.
#[derive(Args, Debug)]
pub struct RevokeArgs {
    /// Node id or identity account
    pub target: Target,

    /// Account losing the capability
    #[arg(value_parser = account_arg)]
    pub account: AccountId,

    /// Capability to revoke
    #[arg(value_enum)]
    pub capability: CapabilityArg,
}

/// Run an `access` subcommand; `check` reports its answer in the exit status.
pub async fn run(ctx: &Ctx, command: AccessCommand) -> Result<ExitCode> {
    match command {
        AccessCommand::Check(args) => check(ctx, args).await,
        AccessCommand::Grant(args) => grant(ctx, args).await.map(|()| ExitCode::SUCCESS),
        AccessCommand::Revoke(args) => revoke(ctx, args).await.map(|()| ExitCode::SUCCESS),
    }
}

async fn check(ctx: &Ctx, args: CheckArgs) -> Result<ExitCode> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let id = args.target.node_id(&at).await?;
    let allowed = at
        .node(id)
        .has_capability(&args.account, args.capability.into())
        .await?;
    if !args.quiet {
        ctx.out.result(
            || if allowed { "allowed" } else { "denied" }.to_string(),
            json!({
                "node": id.0,
                "account": args.account.to_string(),
                "capability": args.capability.name(),
                "allowed": allowed,
            }),
        );
    }
    Ok(if allowed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

async fn grant(ctx: &Ctx, args: GrantArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let grant = Grant::new(args.capability.into(), args.mode.into());
    let tx = cps.node(id).grant(args.account, grant)?;
    let receipt = ctx.submit(&cps, &tx, &signer).await?;
    ctx.done(
        &receipt,
        format!(
            "Granted {} to {} at node {id} ({:?})",
            args.capability.name(),
            args.account,
            args.mode
        ),
        json!({
            "node": id.0,
            "account": args.account.to_string(),
            "capability": args.capability.name(),
            "mode": format!("{:?}", args.mode).to_lowercase(),
        }),
    );
    Ok(())
}

async fn revoke(ctx: &Ctx, args: RevokeArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let tx = cps.node(id).revoke(args.account, args.capability.into())?;
    let receipt = ctx.submit(&cps, &tx, &signer).await?;
    ctx.done(
        &receipt,
        format!(
            "Revoked {} from {} at node {id}",
            args.capability.name(),
            args.account
        ),
        json!({
            "node": id.0,
            "account": args.account.to_string(),
            "capability": args.capability.name(),
        }),
    );
    Ok(())
}
