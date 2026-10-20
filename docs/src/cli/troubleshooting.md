# Troubleshooting

## Connection failed

```bash
cps status
```

shows the endpoint `cps` resolved and whether it answers (exit status `2`: it does not).
Check, in order:

1. Is the node or light client running? `curl -H 'Content-Type: application/json' -d '{"id":1,"jsonrpc":"2.0","method":"system_health"}' http://127.0.0.1:9944`
2. Which endpoint is used? A leftover `ROBONOMICS_WS_URL`, `CPS_PROFILE` or `--profile` can override the
   default; `cps profile show` lists the resolved settings without connecting.
3. Docker: did you publish the port (`-p 9944:9944`)? Pass `--rpc-external` to the node when
   connecting from another host.

## Reads return nothing or `node not found`

- A light client reads **finalized** state only. Right after start it needs a while; wait until
  `cps status` shows the finalized block catching up with the best block.
- Check you are on the intended chain: `cps status` prints the chain name and genesis hash.
- Node ids are chain specific; the ids of your local development chain are not those of the live
  network.

## Account problems

```bash
export ROBONOMICS_SURI=//Alice        # or: cps --suri //Alice ...
```

Writes and `--decrypt` need an account; reads do not. A development account only has funds on a
development chain.

## Access denied

The runtime refused the transaction: the signer is neither the scope owner nor holds the capability.
Check with `cps access check <node> <account> write`, and remember that capabilities granted in
`node` mode do not cover descendants (use `--mode subtree`). Deleting a node is reserved to the scope
owner.

## A write seems to hang

By default writes wait for **finality**, which takes longer than inclusion. Use `--wait best` for
quick feedback, and look at `cps status` to see whether blocks are being produced and finalized.

## Verbose logs

```bash
cps -l debug status
cps -l info,subxt=warn show 1
```
