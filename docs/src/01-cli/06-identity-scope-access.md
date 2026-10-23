# Identities, scopes and access

> **Library twin:** [Identities and addresses](../02-library/06-identity.md) and [Scopes and access](../02-library/07-scopes-and-access.md)

## Identities

An identity binds an account to a node, so the node can be addressed by that account.

```bash
cps create --identity 5Grwva... --meta gateway    # your own account needs no permit
cps identity get 42                              # identity of a node
cps identity set 42 5Grwva...                    # bind (own account, or with --permit)
cps identity clear 42                            # remove the binding
cps show 5Grwva...                               # address a node by its identity account
```

Work with keys locally, with no chain connection:

```bash
cps identity generate                        # a new key (sr25519 by default)
cps identity generate --scheme ed25519
cps identity inspect //Bob                   # account and public key of a URI
cps identity inspect 5FHne... --output json  # of an SS58 account
```

### Permits: identities for other accounts

To bind an account other than your own, that account must sign a permit for the scope of the node.
The signer and the node owner may be different people on different machines:

```bash
# on the device: sign a permit valid for 100 blocks for the scope of node 42
cps identity permit 42 --key "$DEVICE_SURI" --ttl 100 > permit.json

# on the owner's machine: use it
cps identity set 42 --permit permit.json
cps create --parent 42 --permit permit.json
```

`--unsigned` prints the payload to sign instead of signing, for keys that live in a hardware module or
on an offline machine; `--valid-until <BLOCK>` sets an absolute expiry instead of `--ttl`.

## Scopes

Ownership belongs to a scope, not a node: a node resolves to the scope of its nearest ancestor that
roots one.

```bash
cps scope resolve 43              # scope, root, owner and path of a node
cps scope show 1                  # a scope record by id
cps scope create 43               # make a node the root of a nested scope
```

`scope create` refuses a node that already roots a scope.

## Access

```bash
cps access grant 1 5FHne... write                    # the node itself
cps access grant 1 5FHne... write --mode subtree     # and its descendants
cps access check 2 5FHne... write                    # exit status 0 = allowed, 1 = not allowed
cps access check 2 5FHne... write --quiet            # print nothing, only the exit status
cps access revoke 1 5FHne... write
```

Capabilities are `write` and `create-scope`. Because `check` reports its answer in the exit status it
fits conditionals:

```bash
if cps access check "$NODE" "$ACCOUNT" write --quiet; then echo "may write"; fi
```

## Writing from another account

The same signed commands, different `--suri`:

```bash
BOB=$(cps identity inspect //Bob --output json | sed -E 's/.*"address":"([^"]+)".*/\1/')

cps --suri //Bob payload set 2 'denied'                   # fails: access denied
cps --suri //Alice access grant 1 "$BOB" write --mode subtree
cps --suri //Bob payload set 2 'written by Bob'           # succeeds
```

[`examples/shell/cli_journey.sh`](https://github.com/airalab/robins/blob/master/crates/libcps/examples/shell/cli_journey.sh)
runs this whole story end to end.
