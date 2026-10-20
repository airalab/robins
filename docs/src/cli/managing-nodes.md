# Managing nodes and data

> **Library twin:** [Launch your own node and grow a tree](../library/05-your-own-node.md)

Writing needs a chain you can pay on and an account: start the development node from
[the library chapter](../library/05-your-own-node.md) and use the pre-funded `//Alice`.

```bash
export ROBONOMICS_WS_URL=ws://127.0.0.1:9944
export ROBONOMICS_SURI=//Alice
cps status
```

## Create

```bash
# a root node (also creates a scope owned by you)
cps create --meta '{"type":"building","name":"HQ"}' --payload '{"status":"online"}'

# a child node
cps create --parent 0 --meta '{"type":"room"}' --payload '{"temp":"22C"}'
```

The command prints the id of the new node.

| Option | Meaning |
| --- | --- |
| `-p, --parent <TARGET>` | parent node or identity account; omit for a root node |
| `--meta <TEXT>` / `--meta-file <FILE>` | initial metadata; `-` reads standard input |
| `--payload <TEXT>` / `--payload-file <FILE>` | initial payload; `-` reads standard input |
| `--identity <ACCOUNT>` | bind an identity account to the new node |
| `--permit <FILE\|->` | identity permit for an identity other than your own |
| `-r`, `--cipher`, `--scheme` | encrypt the data, see [Encryption](encryption.md) |

## Update

```bash
cps meta set 1 '{"type":"room","name":"Lab"}'
cps payload set 1 'temp=24C'
cat reading.json | cps payload set 1 -         # from standard input
cps payload set 1 --file image.bin             # binary data
cps payload clear 1
```

## Delete

Only nodes without children can be deleted: remove leaves first.

```bash
cps delete 2             # asks for confirmation
cps delete --yes 2       # does not
```

## Waiting and output

A write waits until its block is finalized (`--wait best` for the first inclusion) and then prints the
result. With `--output json` the result is one JSON document on a single line, for example
`{"node":1,"parent":0,"identity":null}` for `create`; diagnostics always go to standard error, so
standard output stays clean for pipes.
