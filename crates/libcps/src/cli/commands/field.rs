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
//! `meta` and `payload` commands: binary-safe get, set, clear and watch.

use crate::cli::{
    args::{DataInputArgs, DecryptArgs, EncryptionArgs, ReadArgs},
    context::Ctx,
    data::{self, json_opt, DataView},
    output::{hex0x, info},
    target::Target,
};
use anyhow::Result;
use clap::{Args, Subcommand};
use libcps::NodeId;
use serde_json::json;
use std::io::Write;

/// Which node field a command works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// Node metadata (configuration).
    Meta,
    /// Node payload (operational data).
    Payload,
}

impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Meta => "meta",
            Self::Payload => "payload",
        }
    }
}

/// Subcommands shared by `meta` and `payload`.
#[derive(Subcommand, Debug)]
pub enum FieldCommand {
    /// Read the data
    Get(GetArgs),
    /// Replace the data; binary input via --file or `-` (standard input)
    Set(SetArgs),
    /// Remove the data
    Clear(TargetArgs),
    /// Follow changes in finalized blocks (JSON Lines with --output json)
    Watch(WatchArgs),
}

/// A bare target.
#[derive(Args, Debug)]
pub struct TargetArgs {
    /// Node id or identity account
    pub target: Target,
}

/// Arguments of `get`.
#[derive(Args, Debug)]
#[command(long_about = "Read the data of a node.

Default output is text (hex for binary data). --raw writes the exact bytes to
standard output, suitable for redirection; --hex prints 0x-prefixed hex.

EXAMPLES:
    cps payload get 42
    cps payload get 42 --raw > batch.bin
    cps payload get 42 --hex --at 0xabc...")]
pub struct GetArgs {
    /// Node id or identity account
    pub target: Target,

    /// Write the raw bytes to standard output
    #[arg(long, conflicts_with = "hex")]
    pub raw: bool,

    /// Print the bytes as 0x-prefixed hex
    #[arg(long)]
    pub hex: bool,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub decrypt: DecryptArgs,
}

/// Arguments of `set`.
#[derive(Args, Debug)]
#[command(
    long_about = "Replace the data of a node. Account targets are resolved to their node first.

EXAMPLES:
    cps payload set 42 'hello'
    cps payload set 42 --file batch.bin
    cat batch.bin | cps payload set 42 -
    cps meta set 42 'private config' --receiver-public 5Grwva..."
)]
pub struct SetArgs {
    /// Node id or identity account
    pub target: Target,

    #[command(flatten)]
    pub input: DataInputArgs,

    #[command(flatten)]
    pub encryption: EncryptionArgs,
}

/// Arguments of `watch`.
#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Node id or identity account (resolved to its node when the watch starts)
    pub target: Target,

    #[command(flatten)]
    pub decrypt: DecryptArgs,
}

/// Run a `meta` or `payload` subcommand.
pub async fn run(ctx: &Ctx, field: Field, command: FieldCommand) -> Result<()> {
    match command {
        FieldCommand::Get(args) => get(ctx, field, args).await,
        FieldCommand::Set(args) => set(ctx, field, args).await,
        FieldCommand::Clear(args) => clear(ctx, field, args).await,
        FieldCommand::Watch(args) => watch(ctx, field, args).await,
    }
}

async fn get(ctx: &Ctx, field: Field, args: GetArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let id = args.target.node_id(&at).await?;
    let node = at.node(id);
    let stored = match field {
        Field::Meta => node.meta().await?,
        Field::Payload => node.payload().await?,
    };
    let bytes = stored
        .map(|b| data::open(&args.decrypt, ctx.suri(), b))
        .transpose()?;

    if args.raw {
        match bytes {
            Some(bytes) => {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
            None => info(&format!("node {id} has no {}", field.name())),
        }
        return Ok(());
    }
    let view = bytes.map(DataView);
    if view.is_none() && !ctx.out.is_json() {
        info(&format!("node {id} has no {}", field.name()));
    }
    ctx.out.result(
        || match &view {
            Some(view) if args.hex => hex0x(&view.0),
            Some(view) => view.human(),
            None => String::new(),
        },
        json_opt(&view),
    );
    Ok(())
}

async fn set(ctx: &Ctx, field: Field, args: SetArgs) -> Result<()> {
    let bytes = data::seal(&args.encryption, ctx.suri(), args.input.read()?)?;
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let node = cps.node(id);
    let tx = match field {
        Field::Meta => node.set_meta(&bytes)?,
        Field::Payload => node.set_payload(&bytes)?,
    };
    let receipt = ctx.submit(&cps, &tx, &signer).await?;
    ctx.done(
        &receipt,
        format!("Node {id}: {} set ({} bytes)", field.name(), bytes.len()),
        json!({ "node": id.0, "bytes": bytes.len() }),
    );
    Ok(())
}

async fn clear(ctx: &Ctx, field: Field, args: TargetArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let node = cps.node(id);
    let tx = match field {
        Field::Meta => node.clear_meta(),
        Field::Payload => node.clear_payload(),
    };
    let receipt = ctx.submit(&cps, &tx, &signer).await?;
    ctx.done(
        &receipt,
        format!("Node {id}: {} cleared", field.name()),
        json!({ "node": id.0 }),
    );
    Ok(())
}

async fn watch(ctx: &Ctx, field: Field, args: WatchArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let id: NodeId = ctx.canonical(&cps, &args.target).await?;
    let node = cps.node(id);
    info(&format!(
        "Watching {} of node {id} (Ctrl-C to stop)",
        field.name()
    ));

    match field {
        Field::Meta => {
            let mut watcher = node.watch_meta().await?;
            loop {
                let update = tokio::select! {
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                    update = watcher.next() => update,
                };
                let Some(update) = update else { return Ok(()) };
                let update = update?;
                emit(
                    ctx,
                    &args.decrypt,
                    id,
                    update.block_number,
                    &update.block_hash.0,
                    update.meta,
                )?;
            }
        }
        Field::Payload => {
            let mut watcher = node.watch_payload().await?;
            loop {
                let update = tokio::select! {
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                    update = watcher.next() => update,
                };
                let Some(update) = update else { return Ok(()) };
                let update = update?;
                emit(
                    ctx,
                    &args.decrypt,
                    id,
                    update.block_number,
                    &update.block_hash.0,
                    update.payload,
                )?;
            }
        }
    }
}

/// Print one watch event.
fn emit(
    ctx: &Ctx,
    decrypt: &DecryptArgs,
    id: NodeId,
    block_number: u64,
    block_hash: &[u8; 32],
    value: Option<Vec<u8>>,
) -> Result<()> {
    let view = value
        .map(|b| data::open(decrypt, ctx.suri(), b).map(DataView))
        .transpose()?;
    ctx.out.event(
        || {
            format!(
                "[#{block_number}] {}",
                view.as_ref()
                    .map_or_else(|| "(cleared)".to_string(), DataView::human)
            )
        },
        json!({
            "node": id.0,
            "block_number": block_number,
            "block_hash": hex0x(block_hash),
            "data": json_opt(&view),
        }),
    );
    Ok(())
}
