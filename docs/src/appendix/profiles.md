# A. Profiles and other conveniences

Once you move between a local chain, a light client and a production endpoint, retyping `--ws-url` gets
old. These conveniences are optional, and none of them changes what you can do.

## Profiles

Recurring settings can be stored in a TOML file as named profiles. The default location is
`$XDG_CONFIG_HOME/cps/config.toml` (`~/.config/cps/config.toml`; the platform configuration directory
on other systems). Select another file with `--config FILE` or `CPS_CONFIG`. A missing *default* file is
fine; a missing file requested explicitly is an error.

```toml
default-profile = "local"

[profiles.local]
url = "ws://127.0.0.1:9944"
output = "human"
wait = "finalized"

[profiles.production]
url = "wss://rpc.example.org"
output = "json"
wait = "finalized"
```

- Profiles hold CLI settings only: `url`, `output` and `wait`. **Never put secret phrases or keys in
  the file**: unknown keys, `suri` included, are rejected.
- Every setting is resolved with one precedence, highest first: command-line option, environment
  variable, selected profile, built-in default.
- The profile itself is chosen by `--profile NAME`, then `CPS_PROFILE`, then `default-profile`.

```bash
cps --profile production show 42
cps profile list               # configured profiles, `yes` marks the default
cps profile show [NAME]        # resolved settings, without connecting
```

The runnable script `09_profiles.sh` walks through it:

```bash
{{#include ../../../crates/libcps/examples/shell/09_profiles.sh}}
```

## Handy defaults

| Convenience | How |
| --- | --- |
| Quick feedback on a dev chain | `export CPS_WAIT=best` |
| Machine-readable results everywhere | `export CPS_OUTPUT=json` |
| One-off endpoint for a command | `cps --ws-url ws://host:9944 ...` |
| Where am I connected? | `cps status` |
| A key for a throwaway device | `cps identity generate` |
| Verbose diagnostics | `cps -l debug ...` |
| Shell completions of options | `cps --help`, `cps <command> --help` |
