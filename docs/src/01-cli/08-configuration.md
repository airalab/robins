# Configuration

Every `cps` setting has a command-line option, an environment variable and an optional profile entry.
They are resolved with one precedence, highest first:

**command-line option → environment variable → selected profile → built-in default**

| Setting | Option | Variable | Default |
| --- | --- | --- | --- |
| RPC endpoint | `--ws-url` | `ROBONOMICS_WS_URL` | `ws://127.0.0.1:9944` |
| Account secret URI | `--suri` | `ROBONOMICS_SURI` | none (reads need no key) |
| Output format | `--output human\|json` | `CPS_OUTPUT` | `human` |
| Transaction wait | `--wait best\|finalized` | `CPS_WAIT` | `finalized` |
| Config file | `--config FILE` | `CPS_CONFIG` | `$XDG_CONFIG_HOME/cps/config.toml` |
| Profile | `--profile NAME` | `CPS_PROFILE` | the file's `default-profile` |
| Log filter | `-l, --log-level` | `RUST_LOG` | `warn` |

All of them are *global*: they are accepted before or after the command.

## Endpoint and account

```bash
export ROBONOMICS_WS_URL=ws://127.0.0.1:9944   # local node, or `cps lc`
export ROBONOMICS_SURI=//Alice                  # development account
```

`--suri` accepts a development URI (`//Alice`) or a seed phrase. It signs transactions (SR25519) and
is the key used for encryption and decryption. **Treat it as a secret:** in production prefer the
environment variable or a secrets manager over a command-line argument, which other users of the
machine can see in the process list.

## `--wait`: best or finalized

By default a write returns when its block is **finalized**, which is safe but takes a while on a real
network. `--wait best` returns once the transaction is in a best block, which a reorganization may
still revert. Use it for fast feedback on a development chain.

## Profiles

Recurring settings (endpoint, output, wait) can live in a TOML file as named profiles.
[Appendix A](../appendix/profiles.md) shows them in full; in short:

```toml
default-profile = "local"

[profiles.local]
url = "ws://127.0.0.1:9944"

[profiles.light]
url = "ws://127.0.0.1:9945"
output = "json"
```

```bash
cps --profile light status
cps profile list
```

Profiles never contain secrets: unknown keys, `suri` included, are rejected.
