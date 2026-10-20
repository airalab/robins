# Exploring the tree

> **Library twin:** [Explore the network](../library/02-explore-network.md) and [Watch the network live](../library/03-watch.md)

Reading needs no key. Point `cps` at a light client (`cps lc`) or any RPC endpoint and explore.

## Targets

Most commands take a `TARGET`: a **numeric node id** or an **identity account** (SS58). An id selects
the node projection, an account the identity projection. Use `node:<id>` or `addr:<account>` to force
the interpretation. Writes always resolve to a node id.

## `show`, `tree`, `children`, `parent`

```bash
cps show 42                 # one node: owner, metadata, payload
cps tree 42                 # the node and everything below it
cps tree 42 --depth 2       # limit the number of levels
cps children 42             # child ids
cps parent 43               # parent id
```

Output of `show`:

```text
[*] CPS Node ID: 0

|--  [O] Owner: 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
|--  [M] Meta: {
  "type": "building",
  "name": "HQ"
}
`--  [P] Payload: {
  "status": "online"
}
```

Add `--decrypt` (with `--suri`) to read data encrypted for you, see [Encryption](encryption.md).

### Reading at a block

Reads use the latest finalized block. Pin a read to another block with `--at <BLOCK_HASH>`, on every
read command, to look at the network as it was or to take several consistent readings:

```bash
cps show 42 --at 0x1234...
```

### Reading data

```bash
cps payload get 42           # as text
cps payload get 42 --hex     # 0x-prefixed hex
cps payload get 42 --raw > data.bin   # raw bytes on stdout
cps meta get 42
```

## Following changes

`watch` prints a line for every finalized block that changes a field, until you press `Ctrl+C`:

```bash
cps payload watch 42
cps --output json payload watch 42     # one JSON object per line (JSON Lines)
```

An identity account works as target too: it is resolved to its node when the watch starts.

## The story so far

```bash
cps lc &                     # join the network
cps status                   # wait for it to be synced
cps tree <SENSOR_SOCIAL_ROOT> --depth 2
cps payload watch <SENSOR_NODE>
```
