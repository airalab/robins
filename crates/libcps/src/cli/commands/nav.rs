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
//! Navigation commands: `show`, `tree`, `parent` and `children`.
//!
//! The target selects the projection: a node target answers in `NodeId`
//! space, an account target in identity space (failing explicitly when a node
//! of the answer has no identity).

use crate::cli::{
    args::{DecryptArgs, ReadArgs},
    context::Ctx,
    data::{self, human_opt, json_opt, DataView},
    output::fields,
    target::{Target, Tree},
};
use anyhow::Result;
use clap::Args;
use serde_json::{json, Value};

/// Arguments of commands that take a target and a read selection.
#[derive(Args, Debug)]
pub struct NavArgs {
    /// Node id or identity account
    pub target: Target,

    #[command(flatten)]
    pub read: ReadArgs,
}

/// Arguments of `show`.
#[derive(Args, Debug)]
#[command(long_about = "Display a single node, not its subtree.

The target decides the projection: a node id shows node-space relations, an
account shows identity-space relations.

EXAMPLES:
    cps show 42
    cps show 5FHneWLhCJ6RHqVA4MYnq1R8HWtQTJJPn4BdhgQvJMtF6Kcz
    cps --output json show 42 --at 0xabc...")]
pub struct ShowArgs {
    /// Node id or identity account
    pub target: Target,

    #[command(flatten)]
    pub read: ReadArgs,

    #[command(flatten)]
    pub decrypt: DecryptArgs,
}

/// Arguments of `tree`.
#[derive(Args, Debug)]
#[command(long_about = "Display the subtree below a target.

EXAMPLES:
    cps tree 1
    cps tree 5Grwva... --depth 2")]
pub struct TreeArgs {
    /// Node id or identity account
    pub target: Target,

    /// Maximum number of levels below the target
    #[arg(long)]
    pub depth: Option<usize>,

    #[command(flatten)]
    pub read: ReadArgs,
}

/// `cps show`.
pub async fn show(ctx: &Ctx, args: ShowArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let node_id = args.target.node_id(&at).await?;
    let node = at.node(node_id);

    let snapshot = node
        .snapshot()
        .await?
        .ok_or(libcps::Error::NodeNotFound(node_id))?;
    let scope = node.resolve_scope().await?;
    let parent = args.target.parent(&at).await?;
    let view = |bytes: Option<Vec<u8>>| -> Result<Option<DataView>> {
        bytes
            .map(|b| data::open(&args.decrypt, ctx.suri(), b).map(DataView))
            .transpose()
    };
    let meta = view(snapshot.meta)?;
    let payload = view(snapshot.payload)?;
    let dash = |t: Option<Target>| t.map_or_else(|| "-".to_string(), |t| t.to_string());

    let (mut rows, mut json) = match &args.target {
        Target::Node(id) => (
            vec![
                ("Node", id.to_string()),
                (
                    "Identity",
                    snapshot.identity.map_or("-".into(), |a| a.to_string()),
                ),
            ],
            json!({
                "node": id.0,
                "identity": snapshot.identity.map(|a| a.to_string()),
            }),
        ),
        Target::Address(account) => (
            vec![
                ("Address", account.to_string()),
                ("Node", node_id.to_string()),
            ],
            json!({ "address": account.to_string(), "node": node_id.0 }),
        ),
    };
    rows.extend([
        ("Parent", dash(parent.clone())),
        ("Scope", scope.id.to_string()),
        ("Owner", scope.owner.to_string()),
        ("Meta", human_opt(&meta)),
        ("Payload", human_opt(&payload)),
    ]);
    let extra = json!({
        "parent": parent.as_ref().map(Target::json),
        "scope": scope.id.0,
        "owner": scope.owner.to_string(),
        "meta": json_opt(&meta),
        "payload": json_opt(&payload),
    });
    if let (Value::Object(base), Value::Object(extra)) = (&mut json, extra) {
        base.extend(extra);
    }
    ctx.out.result(|| fields(&rows), json);
    Ok(())
}

/// `cps tree`.
pub async fn tree(ctx: &Ctx, args: TreeArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let tree = Tree::read(&at, args.target, args.depth).await?;
    ctx.out.result(|| tree.render(), tree.json());
    Ok(())
}

/// `cps parent`.
pub async fn parent(ctx: &Ctx, args: NavArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let parent = args.target.parent(&at).await?;
    ctx.out.result(
        || {
            parent
                .as_ref()
                .map_or_else(|| "-".into(), Target::to_string)
        },
        parent.as_ref().map_or(Value::Null, Target::json),
    );
    Ok(())
}

/// `cps children`.
pub async fn children(ctx: &Ctx, args: NavArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let children = args.target.children(&at).await?;
    ctx.out.result(
        || {
            children
                .iter()
                .map(Target::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        },
        Value::Array(children.iter().map(Target::json).collect()),
    );
    Ok(())
}
