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
//! Node lifecycle commands: `create` and `delete`.

use crate::cli::{
    args::{account_arg, EncryptionArgs, PermitInputArgs},
    commands::identity::assignment,
    context::{confirm, Ctx},
    data::{self, read_arg, read_file},
    target::Target,
};
use anyhow::{bail, Result};
use clap::Args;
use libcps::{AccountId, CreateNode};
use serde_json::json;
use std::path::PathBuf;

/// Arguments of `create`.
#[derive(Args, Debug)]
#[command(
    long_about = "Create a node: a child of --parent, or a root node (a new scope owned by the signer).

DATA arguments are text, or `-` for standard input; --meta-file/--payload-file
read binary files. Only one argument may read standard input.

A permit lets another account be the identity of the new child; it carries the
identity, so --identity is not needed.

EXAMPLES:
    cps create --meta '{\"type\":\"sensor\"}' --payload '22.5C'
    cps create --parent 42 --payload 'operational data'
    cps create --parent 5FHneW... --payload-file batch.bin
    cps create --parent 42 --permit device.permit.json
    cps create --parent 42 --payload 'secret' --receiver-public 5Grwva..."
)]
pub struct CreateArgs {
    /// Parent node id or identity account (omit for a root node)
    #[arg(short = 'p', long)]
    pub parent: Option<Target>,

    /// Initial metadata as text, or `-` for standard input
    #[arg(long, conflicts_with = "meta_file")]
    pub meta: Option<String>,

    /// Initial metadata from a file
    #[arg(long, value_name = "FILE")]
    pub meta_file: Option<PathBuf>,

    /// Initial payload as text, or `-` for standard input
    #[arg(long, conflicts_with = "payload_file")]
    pub payload: Option<String>,

    /// Initial payload from a file
    #[arg(long, value_name = "FILE")]
    pub payload_file: Option<PathBuf>,

    /// Identity account of the new node (the signer's own account needs no permit)
    #[arg(long, value_parser = account_arg)]
    pub identity: Option<AccountId>,

    #[command(flatten)]
    pub permit: PermitInputArgs,

    #[command(flatten)]
    pub encryption: EncryptionArgs,
}

/// Arguments of `delete`.
#[derive(Args, Debug)]
#[command(long_about = "Delete a node. It must have no children.

EXAMPLES:
    cps delete 5
    cps delete 5FHneW... --yes")]
pub struct DeleteArgs {
    /// Node id or identity account
    pub target: Target,

    /// Do not ask for confirmation
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// `cps create`.
pub async fn create(ctx: &Ctx, args: CreateArgs) -> Result<()> {
    let meta = match (&args.meta, &args.meta_file) {
        (Some(data), _) => Some(read_arg(data)?),
        (None, Some(file)) => Some(read_file(file)?),
        (None, None) => None,
    };
    let payload = match (&args.payload, &args.payload_file) {
        (Some(data), _) => Some(read_arg(data)?),
        (None, Some(file)) => Some(read_file(file)?),
        (None, None) => None,
    };
    let seal = |bytes: Option<Vec<u8>>| {
        bytes
            .map(|b| data::seal(&args.encryption, ctx.suri(), b))
            .transpose()
    };
    let (meta, payload) = (seal(meta)?, seal(payload)?);

    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let parent = match &args.parent {
        Some(target) => Some(ctx.canonical(&cps, target).await?),
        None => None,
    };
    if parent.is_none() && args.permit.permit.is_some() {
        bail!("a permit needs --parent: a root node can only get the signer as identity");
    }
    let identity = assignment(&cps, args.identity, args.permit.permit.as_deref(), parent).await?;

    let params = CreateNode {
        meta,
        payload,
        identity,
    };
    let tx = match parent {
        Some(parent) => cps.node(parent).create_child(params)?,
        None => libcps::create_root(params)?,
    };
    let receipt = ctx.submit(&cps, &tx, &signer).await?;
    let id = receipt.result;
    ctx.done(
        &receipt,
        format!("Node {id} created"),
        json!({
            "node": id.0,
            "parent": parent.map(|p| p.0),
            "identity": identity.map(|i| i.account.to_string()),
        }),
    );
    Ok(())
}

/// `cps delete`.
pub async fn delete(ctx: &Ctx, args: DeleteArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let node = cps.node(id);

    let children = node.children().await?;
    if !children.is_empty() {
        bail!(
            "cannot delete node {id}: it has {} children, delete them first",
            children.len()
        );
    }
    if !confirm(&format!("Delete node {id}?"), args.yes)? {
        crate::cli::output::info("Deletion cancelled");
        return Ok(());
    }
    let receipt = ctx.submit(&cps, &node.delete(), &signer).await?;
    ctx.done(
        &receipt,
        format!("Node {id} deleted"),
        json!({ "node": id.0 }),
    );
    Ok(())
}
