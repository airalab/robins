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
//! CPS CLI - command-line interface for the Robonomics CPS pallet.
//!
//! Commands operate on CPS resources addressed by a common `<TARGET>`: a numeric
//! node id or an identity account. See [`cli`] for the shared building blocks.

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::{io::IsTerminal, process::ExitCode};

mod cli;

use cli::{
    args::{ConfigArgs, ConnectionArgs, OutputArgs},
    commands::{
        access::{self, AccessCommand},
        field::{self, Field, FieldCommand},
        identity::{self, IdentityCommand},
        nav::{self, NavArgs, ShowArgs, TreeArgs},
        node::{self, CreateArgs, DeleteArgs},
        profile::{self, ProfileCommand},
        scope::{self, ScopeCommand},
        status,
    },
    config::{self, CliSettings},
    context::Ctx,
    light_client,
    output::Output,
};

#[derive(Parser)]
#[command(name = "cps")]
#[command(version, about = "Robonomics Cyber-Physical System toolbox", long_about = None)]
#[command(
    after_help = "TARGET is a numeric node id or an identity account (SS58). An id selects the\nnode projection, an account the identity projection; `node:<id>` and\n`addr:<account>` force the interpretation. Writes always resolve to a node id."
)]
#[command(before_help = r#"
╔══════════════════════════════════════════════════════╗
║                                                      ║
║     ██╗     ██╗██████╗  ██████╗██████╗ ███████╗      ║
║     ██║     ██║██╔══██╗██╔════╝██╔══██╗██╔════╝      ║
║     ██║     ██║██████╔╝██║     ██████╔╝███████╗      ║
║     ██║     ██║██╔══██╗██║     ██╔═══╝ ╚════██║      ║
║     ███████╗██║██████╔╝╚██████╗██║     ███████║      ║
║     ╚══════╝╚═╝╚═════╝  ╚═════╝╚═╝     ╚══════╝      ║
║                                                      ║
║     Cyber-Physical System - Robonomics Network       ║
║                                                      ║
╚══════════════════════════════════════════════════════╝
"#)]
struct Cli {
    #[command(flatten)]
    connection: ConnectionArgs,

    #[command(flatten)]
    output: OutputArgs,

    #[command(flatten)]
    config: ConfigArgs,

    /// Log filter: a level (off, error, warn, info, debug, trace) or tracing
    /// directives such as `info,subxt=warn`
    #[arg(
        short = 'l',
        long,
        global = true,
        env = "RUST_LOG",
        default_value = "warn"
    )]
    log_level: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Display one node (not its subtree)
    Show(ShowArgs),
    /// Display the subtree below a node
    Tree(TreeArgs),
    /// Print the parent, in the projection of the target
    Parent(NavArgs),
    /// Print the children, in the projection of the target
    Children(NavArgs),
    /// Create a root or child node
    Create(CreateArgs),
    /// Delete a node without children
    Delete(DeleteArgs),
    /// Node metadata (configuration)
    Meta {
        #[command(subcommand)]
        command: FieldCommand,
    },
    /// Node payload (operational data)
    Payload {
        #[command(subcommand)]
        command: FieldCommand,
    },
    /// Node identities and identity keys
    #[command(visible_alias = "id")]
    Identity {
        #[command(subcommand)]
        command: IdentityCommand,
    },
    /// Access scopes
    Scope {
        #[command(subcommand)]
        command: ScopeCommand,
    },
    /// Capabilities of accounts in a scope
    Access {
        #[command(subcommand)]
        command: AccessCommand,
    },
    /// Show the effective configuration and the live state of the connected chain
    #[command(
        long_about = "Show the profile, config file and RPC endpoint the CLI resolved
(command line > environment > profile > built-in defaults), then query the
endpoint for the chain name, genesis hash, runtime version, best and finalized
blocks and the synchronization state. The effective configuration is shown even
when the endpoint cannot be reached; the exit status is then 2.

Works the same against a full node, a remote RPC service and `cps lc`."
    )]
    Status,
    /// Configuration profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Run the embedded light client as a local JSON-RPC endpoint
    #[command(
        visible_alias = "lc",
        long_about = "Run the embedded smoldot light client in the foreground and serve a local
WebSocket JSON-RPC endpoint that other cps commands (and any Substrate RPC
client) can connect to. Stop it with Ctrl-C.

The light client keeps no state: every start synchronizes from scratch.

EXAMPLES:
    # Listen on 127.0.0.1:9944 (the default endpoint of other commands)
    cps lc

    # Listen on another port
    cps lc --listen 127.0.0.1:9955

    # Use it from another terminal
    cps --ws-url ws://127.0.0.1:9955 show 0"
    )]
    LightClient(LightClientArgs),
}

#[derive(Args)]
struct LightClientArgs {
    /// Address of the local RPC endpoint
    #[arg(long, default_value = light_client::DEFAULT_LISTEN)]
    listen: String,

    /// Allow a non-loopback listen address. The endpoint has no authentication
    #[arg(long)]
    allow_external: bool,
}

/// Exit status: 0 success, 1 a negative answer (`access check`), 2 any error.
#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    if !std::io::stderr().is_terminal() {
        colored::control::set_override(false);
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&cli.log_level)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    match run(cli).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let resolved = config::resolve_from_env(CliSettings {
        config: cli.config.config,
        profile: cli.config.profile,
        url: cli.connection.ws_url,
        output: cli.output.output,
        wait: cli.output.wait,
    })?;
    let ctx = Ctx {
        out: Output::new(resolved.effective.output),
        cfg: resolved.effective,
        file: resolved.file,
        suri: cli.connection.suri,
    };
    let ctx = &ctx;
    match cli.command {
        Command::Show(args) => nav::show(ctx, args).await?,
        Command::Tree(args) => nav::tree(ctx, args).await?,
        Command::Parent(args) => nav::parent(ctx, args).await?,
        Command::Children(args) => nav::children(ctx, args).await?,
        Command::Create(args) => node::create(ctx, args).await?,
        Command::Delete(args) => node::delete(ctx, args).await?,
        Command::Meta { command } => field::run(ctx, Field::Meta, command).await?,
        Command::Payload { command } => field::run(ctx, Field::Payload, command).await?,
        Command::Identity { command } => identity::run(ctx, command).await?,
        Command::Scope { command } => scope::run(ctx, command).await?,
        Command::Access { command } => return access::run(ctx, command).await,
        Command::Status => return status::run(ctx).await,
        Command::Profile { command } => profile::run(ctx, command)?,
        Command::LightClient(args) => {
            light_client::execute(light_client::node_config(&args.listen, args.allow_external))
                .await?
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("cps").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn command_tree_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn light_client_command_and_alias_use_the_default_listener() {
        for name in ["light-client", "lc"] {
            match parse(&[name]).command {
                Command::LightClient(args) => {
                    assert_eq!(args.listen, "127.0.0.1:9944");
                    assert!(!args.allow_external);
                }
                _ => panic!("`cps {name}` is not the light client command"),
            }
        }
    }

    #[test]
    fn light_client_listen_is_configurable() {
        match parse(&["lc", "--listen", "127.0.0.1:9955"]).command {
            Command::LightClient(args) => assert_eq!(args.listen, "127.0.0.1:9955"),
            _ => panic!("not the light client command"),
        }
    }

    #[test]
    fn ws_url_is_unset_unless_given() {
        assert_eq!(parse(&["show", "0"]).connection.ws_url, None);
        assert_eq!(
            parse(&["--ws-url", "ws://h:1", "show", "0"])
                .connection
                .ws_url,
            Some("ws://h:1".into())
        );
    }

    #[test]
    fn config_selection_options_are_global() {
        let cli = parse(&[
            "status",
            "--profile",
            "dev",
            "--config",
            "c.toml",
            "--wait",
            "best",
        ]);
        assert_eq!(cli.config.profile.as_deref(), Some("dev"));
        assert_eq!(cli.config.config, Some("c.toml".into()));
        assert_eq!(cli.output.wait, Some(config::WaitPolicy::Best));
    }

    #[test]
    fn profile_and_status_commands_parse() {
        assert!(matches!(parse(&["status"]).command, Command::Status));
        assert!(matches!(
            parse(&["profile", "list"]).command,
            Command::Profile { .. }
        ));
        assert!(matches!(
            parse(&["profile", "show", "dev"]).command,
            Command::Profile { .. }
        ));
    }

    #[test]
    fn id_is_an_alias_of_identity() {
        for name in ["identity", "id"] {
            assert!(matches!(
                parse(&[name, "generate"]).command,
                Command::Identity { .. }
            ));
        }
    }

    #[test]
    fn global_options_work_before_and_after_the_command() {
        let a = parse(&["--output", "json", "show", "42"]);
        let b = parse(&["show", "42", "--output", "json"]);
        assert!(a.output.output == b.output.output);
        assert_eq!(a.output.output, Some(cli::output::OutputFormat::Json));
    }

    #[test]
    fn legacy_commands_are_removed() {
        for legacy in ["set-meta", "set-payload", "remove"] {
            assert!(Cli::try_parse_from(["cps", legacy, "1"]).is_err());
        }
    }

    #[test]
    fn writes_do_not_accept_at() {
        assert!(Cli::try_parse_from(["cps", "payload", "set", "1", "x", "--at", "0x00"]).is_err());
        assert!(Cli::try_parse_from(["cps", "delete", "1", "--at", "0x00"]).is_err());
    }
}
