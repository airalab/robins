# Scripting and automation

`cps` is designed to be a well-behaved citizen of shell pipelines.

## Output streams

- **Standard output** carries results only: human text, a JSON document per result with
  `--output json`, JSON Lines for watches, or raw bytes for `payload get --raw`.
- **Standard error** carries diagnostics, progress and prompts, so `cps ... > file` never captures
  noise.

```bash
cps --output json create --meta '{"type":"sensor"}'      # {"node":7,"parent":null,"identity":null}
cps --output json tree 7
cps --output json payload watch 7 | while read -r line; do echo "update: $line"; done
```

Parse JSON with `jq`, or with `sed` for simple fields as the bundled scripts do.

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | success; for `access check`: access granted |
| `1` | only `access check`: access **not** granted |
| `2` | any error, e.g. an unreachable endpoint (also `cps status`), access denied on a write, bad input |

## Handy patterns

```bash
# create a node and keep its id
NODE=$(cps --output json create --payload 'hello' | sed -E 's/.*"node":([0-9]+).*/\1/')

# publish a reading every minute
while sleep 60; do
  cps --wait best payload set "$NODE" "$(date -u +%FT%TZ) $(read_sensor)"
done

# store a file, read it back byte for byte
cps payload set "$NODE" --file photo.jpg
cps payload get "$NODE" --raw > photo-copy.jpg

# non-interactive deletion
cps delete --yes "$NODE"
```

## Environment for CI and containers

```bash
export ROBONOMICS_WS_URL=ws://node.internal:9944
export ROBONOMICS_SURI="$SECRET_FROM_VAULT"
export CPS_OUTPUT=json
export CPS_WAIT=best
```

## Ready-made scripts

The repository ships runnable scripts in
[`crates/libcps/examples/shell`](https://github.com/airalab/robins/tree/master/crates/libcps/examples/shell):

| Script | Shows |
| --- | --- |
| `cli_journey.sh` | create a tree, write, grant access, write as another account, encrypt |
| `09_profiles.sh` | profiles and configuration resolution |
