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
//! `identity` commands: assignment of identities to nodes and local identity
//! key tooling (generate, inspect, permit).

use crate::cli::{
    args::{account_arg, PermitInputArgs, ReadArgs},
    commands::{field::TargetArgs, nav::NavArgs},
    context::Ctx,
    data::{read_file, read_stdin},
    output::{fields, hex0x, warn},
    target::Target,
};
use anyhow::{anyhow, bail, Result};
use clap::{Args, Subcommand, ValueEnum};
use libcps::{
    identity::{IdentityKey, IdentityScheme},
    permit::{IDENTITY_PERMIT_DOMAIN, PERMIT_ARTIFACT_VERSION},
    AccountId, Cps, IdentityAssignment, NodeId, PermitArtifact, ScopeId,
};
use serde_json::{json, Value};
use std::{path::Path, str::FromStr};

/// Signature scheme selection of identity keys.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum SchemeArg {
    /// Schnorrkel SR25519 (default)
    #[default]
    #[value(alias = "sr")]
    Sr25519,
    /// ED25519
    #[value(alias = "ed")]
    Ed25519,
    /// ECDSA over secp256k1
    Ecdsa,
}

impl From<SchemeArg> for IdentityScheme {
    fn from(scheme: SchemeArg) -> Self {
        match scheme {
            SchemeArg::Sr25519 => Self::Sr25519,
            SchemeArg::Ed25519 => Self::Ed25519,
            SchemeArg::Ecdsa => Self::Ecdsa,
        }
    }
}

/// Subcommands of `identity` (alias `id`).
#[derive(Subcommand, Debug)]
pub enum IdentityCommand {
    /// Show the identity account of a node
    Get(NavArgs),
    /// Bind an identity to a node, directly or with a permit
    Set(SetArgs),
    /// Remove the identity of a node
    Clear(TargetArgs),
    /// Generate a new identity key locally
    Generate(GenerateArgs),
    /// Inspect local key material or an account, offline
    Inspect(InspectArgs),
    /// Issue an identity permit for the scope of a node
    Permit(PermitArgs),
}

/// Arguments of `identity set`.
#[derive(Args, Debug)]
#[command(long_about = "Bind an identity to a node.

Either name the account (the signer's own account needs no permit) or pass a
permit artifact, which carries the identity.

EXAMPLES:
    cps identity set 42 5FHneW...
    cps identity set 42 --permit device.permit.json
    cps identity permit 42 --key \"$DEVICE_SURI\" | cps identity set 42 --permit -")]
pub struct SetArgs {
    /// Node id or identity account of the node
    pub target: Target,

    /// Identity account to bind; optional when --permit names it
    #[arg(value_parser = account_arg, required_unless_present = "permit")]
    pub account: Option<AccountId>,

    #[command(flatten)]
    pub permit: PermitInputArgs,
}

/// Arguments of `identity generate`.
#[derive(Args, Debug)]
#[command(
    long_about = "Generate a new identity key locally. Nothing is submitted to the chain.

The secret phrase is printed once and is the only copy: store it safely.

EXAMPLES:
    cps identity generate
    cps id generate --scheme ed25519
    cps --output json identity generate --scheme ecdsa"
)]
pub struct GenerateArgs {
    /// Signature scheme of the new key
    #[arg(long, value_enum, ignore_case = true, default_value_t = SchemeArg::Sr25519)]
    pub scheme: SchemeArg,
}

/// Arguments of `identity inspect`.
#[derive(Args, Debug)]
#[command(
    long_about = "Inspect local identity material without touching the chain.

KEY is a secret URI (//Alice, a mnemonic phrase, a 0x seed) or a bare SS58
account. A secret is interpreted in --scheme because the same URI derives a
different key in every scheme. A bare account carries no scheme, so none is
reported.

EXAMPLES:
    cps identity inspect '//Alice'
    cps identity inspect '//Alice' --scheme ecdsa
    cps identity inspect 5Grwva..."
)]
pub struct InspectArgs {
    /// Secret URI or SS58 account
    pub key: String,

    /// Scheme in which a secret URI is interpreted
    #[arg(long, value_enum, ignore_case = true, default_value_t = SchemeArg::Sr25519)]
    pub scheme: SchemeArg,
}

/// Arguments of `identity permit`.
#[derive(Args, Debug)]
#[command(long_about = "Issue an identity permit for the scope of TARGET.

The scope and chain context are resolved from the chain. The permit is bound to
that scope and chain and expires at a block: --ttl counts blocks from the latest
finalized block (default 600, capped by the runtime limit), --valid-until names
the block directly. The permit artifact is printed as JSON, ready to pipe into
`identity set --permit -` or `create --permit -`.

--unsigned prints the bytes to sign instead, for external or hardware signers.

EXAMPLES:
    cps identity permit 42 --key '//Device' --ttl 600
    cps identity permit 42 --key '//Device' --valid-until 12345678
    cps identity permit 42 --unsigned")]
pub struct PermitArgs {
    /// Node id or identity account whose scope the permit is for
    pub target: Target,

    /// Secret URI of the identity key that signs
    #[arg(long, value_name = "SURI", required_unless_present = "unsigned")]
    pub key: Option<String>,

    /// Scheme of the identity key
    #[arg(long, value_enum, ignore_case = true, default_value_t = SchemeArg::Sr25519)]
    pub scheme: SchemeArg,

    /// Validity in blocks from the latest finalized block
    #[arg(long, value_name = "BLOCKS", conflicts_with = "valid_until")]
    pub ttl: Option<u32>,

    /// Last block at which the permit is valid
    #[arg(long, value_name = "BLOCK")]
    pub valid_until: Option<u32>,

    /// Emit the signing payload instead of signing; needs no key
    #[arg(long, conflicts_with = "key")]
    pub unsigned: bool,

    #[command(flatten)]
    pub read: ReadArgs,
}

const DEFAULT_TTL: u32 = 600;

/// Run an `identity` subcommand.
pub async fn run(ctx: &Ctx, command: IdentityCommand) -> Result<()> {
    match command {
        IdentityCommand::Get(args) => get(ctx, args).await,
        IdentityCommand::Set(args) => set(ctx, args).await,
        IdentityCommand::Clear(args) => clear(ctx, args).await,
        IdentityCommand::Generate(args) => generate(ctx, args),
        IdentityCommand::Inspect(args) => inspect(ctx, args),
        IdentityCommand::Permit(args) => permit(ctx, args).await,
    }
}

/// Parse a permit artifact from a file, or standard input for `-`.
pub fn load_permit(source: &str) -> Result<PermitArtifact> {
    let bytes = if source == "-" {
        read_stdin()?
    } else {
        read_file(Path::new(source))?
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| anyhow!("permit is not valid UTF-8"))?;
    Ok(PermitArtifact::from_json(text)?)
}

/// Reject a permit that cannot work for a node in `scope_node`'s scope, before
/// spending a transaction on it.
async fn check_permit(cps: &Cps, permit: &PermitArtifact, scope_node: NodeId) -> Result<()> {
    if !permit.verify() {
        bail!(
            "permit signature is not valid for identity {}",
            permit.identity
        );
    }
    if permit.payload.genesis_hash != cps.genesis_hash() {
        bail!("permit was issued for a different chain (genesis hash mismatch)");
    }
    let at = cps.finalized().await?;
    let scope = at.node(scope_node).resolve_scope().await?.id;
    if permit.payload.scope != scope {
        bail!(
            "permit was issued for scope {}, but node {scope_node} is in scope {scope}",
            permit.payload.scope
        );
    }
    if permit.payload.valid_until < at.block_number() {
        bail!(
            "permit expired at block {} (current block {})",
            permit.payload.valid_until,
            at.block_number()
        );
    }
    Ok(())
}

/// Identity assignment from an optional account and permit.
///
/// A permit carries its identity, so an explicit `identity` must agree with it.
/// `scope_node` is the node whose scope the new identity will be used in.
pub async fn assignment(
    cps: &Cps,
    identity: Option<AccountId>,
    permit: Option<&str>,
    scope_node: Option<NodeId>,
) -> Result<Option<IdentityAssignment>> {
    match (permit, identity) {
        (Some(source), identity) => {
            let artifact = load_permit(source)?;
            if identity.is_some_and(|account| account != artifact.identity) {
                bail!("--identity differs from the identity in the permit");
            }
            let scope_node = scope_node
                .ok_or_else(|| anyhow!("a permit can only be used for a node below a parent"))?;
            check_permit(cps, &artifact, scope_node).await?;
            Ok(Some(artifact.into_assignment()))
        }
        (None, Some(account)) => Ok(Some(IdentityAssignment::new(account))),
        (None, None) => Ok(None),
    }
}

async fn get(ctx: &Ctx, args: NavArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let id = args.target.node_id(&at).await?;
    let identity = at
        .node(id)
        .identity()
        .await?
        .ok_or_else(|| anyhow!("node {id} has no identity"))?;
    ctx.out.result(
        || identity.to_string(),
        json!({ "node": id.0, "identity": identity.to_string() }),
    );
    Ok(())
}

async fn set(ctx: &Ctx, args: SetArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let assignment = assignment(&cps, args.account, args.permit.permit.as_deref(), Some(id))
        .await?
        .expect("an account or a permit is required");
    let tx = cps.node(id).set_identity(assignment)?;
    let receipt = ctx.submit(&cps, &tx, &signer).await?;
    ctx.done(
        &receipt,
        format!("Node {id}: identity set to {}", assignment.account),
        json!({ "node": id.0, "identity": assignment.account.to_string() }),
    );
    Ok(())
}

async fn clear(ctx: &Ctx, args: TargetArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let signer = ctx.signer()?;
    let id = ctx.canonical(&cps, &args.target).await?;
    let receipt = ctx
        .submit(&cps, &cps.node(id).clear_identity(), &signer)
        .await?;
    ctx.done(
        &receipt,
        format!("Node {id}: identity cleared"),
        json!({ "node": id.0 }),
    );
    Ok(())
}

fn generate(ctx: &Ctx, args: GenerateArgs) -> Result<()> {
    let scheme = IdentityScheme::from(args.scheme);
    let generated = IdentityKey::generate(scheme)?;
    let key = &generated.key;
    warn("The secret below is shown once and is the only copy. Keep it private.");
    ctx.out.result(
        || {
            fields(&[
                ("Scheme", scheme.to_string()),
                ("Address", key.account_id().to_string()),
                ("Public", hex0x(&key.public())),
                ("Secret", generated.phrase.clone()),
            ])
        },
        json!({
            "scheme": scheme.to_string(),
            "address": key.account_id().to_string(),
            "public": hex0x(&key.public()),
            "secret": generated.phrase,
        }),
    );
    Ok(())
}

fn inspect(ctx: &Ctx, args: InspectArgs) -> Result<()> {
    let (scheme, account, public): (Option<IdentityScheme>, AccountId, Vec<u8>) =
        match AccountId::from_str(&args.key) {
            Ok(account) => (None, account, account.0.to_vec()),
            Err(_) => {
                let scheme = IdentityScheme::from(args.scheme);
                let key = IdentityKey::from_suri(scheme, &args.key)?;
                (Some(scheme), key.account_id(), key.public())
            }
        };
    let scheme_text = scheme.map_or_else(|| "unknown".to_string(), |s| s.to_string());
    ctx.out.result(
        || {
            fields(&[
                ("Address", account.to_string()),
                ("Public", hex0x(&public)),
                ("Scheme", scheme_text.clone()),
            ])
        },
        json!({
            "address": account.to_string(),
            "public": hex0x(&public),
            "scheme": scheme.map(|s| s.to_string()),
        }),
    );
    Ok(())
}

async fn permit(ctx: &Ctx, args: PermitArgs) -> Result<()> {
    let cps = ctx.cps().await?;
    let at = ctx.reader(&cps, &args.read).await?;
    let node = args.target.node_id(&at).await?;
    let scope: ScopeId = at.node(node).resolve_scope().await?.id;

    let now = at.block_number();
    let max_ttl = at.max_identity_permit_ttl()?;
    let valid_until = match (args.valid_until, args.ttl) {
        (Some(block), _) => block,
        (None, ttl) => {
            let ttl = ttl.unwrap_or(DEFAULT_TTL.min(max_ttl));
            now.checked_add(ttl)
                .ok_or_else(|| anyhow!("ttl overflows the block number"))?
        }
    };
    let payload = cps.identity_permit_payload(scope, valid_until).await?;

    let json: Value = if args.unsigned {
        json!({
            "version": PERMIT_ARTIFACT_VERSION,
            "domain": String::from_utf8_lossy(IDENTITY_PERMIT_DOMAIN),
            "genesis_hash": hex0x(&payload.genesis_hash.0),
            "scope": payload.scope.0,
            "valid_until": payload.valid_until,
            "signing_bytes": hex0x(&payload.signing_bytes()),
        })
    } else {
        let suri = args.key.as_deref().expect("clap requires --key");
        let key = IdentityKey::from_suri(args.scheme.into(), suri)?;
        serde_json::from_str(&payload.sign(&key).to_json())?
    };
    // The artifact is a pipeline format: always JSON, compact for `--output json`.
    let pretty = serde_json::to_string_pretty(&json).expect("plain data serializes");
    ctx.out.result(|| pretty, json);
    Ok(())
}
