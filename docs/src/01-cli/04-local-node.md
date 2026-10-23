# Run a local development node

> **Library twin:** [Launch your own node and grow a tree](../02-library/05-your-own-node.md)

Writing needs a chain you can pay on and an account. The live network is read-only for a newcomer,
so start your own Robonomics chain on your machine, with pre-funded accounts, instant block
production and a throwaway database. The next chapters write to it.

## Launch the node

There is no standalone Robonomics node binary. The
[airalab/robonomics](https://github.com/airalab/robonomics) repository builds the runtime and ships
a helper that runs it with the generic `polkadot-omni-node` as a single-node `--dev` chain. Its
developer guide ([Running a Development Chain](https://github.com/airalab/robonomics/blob/master/DEVELOPER_GUIDE.md#running-a-development-chain))
has the one-liners. In a checkout of that repository:

```bash
# build the runtime, generate the dev chain spec and bake both into a Docker image
nix develop -c ./scripts/development-chain.sh docker

# the script leaves the image untagged: tag the newest one and run it
docker tag "$(docker images -q | head -n 1)" robonomics-dev
docker run --rm -p 9944:9944 robonomics-dev
```

You get:

- JSON-RPC and WebSocket on `ws://127.0.0.1:9944`;
- block production without any session key setup;
- pre-funded development accounts `//Alice`, `//Bob`, `//Charlie`, ...;
- an ephemeral database: stop the container and the chain is gone.

Arguments after the image name replace its default command, so repeat the defaults when adding
flags: `docker run --rm -p 9944:9944 robonomics-dev --chain=/dev.json --dev --rpc-external`.

Prefer no Docker? `nix develop -c ./scripts/development-chain.sh run` runs the same chain as a
local process, and `cargo install staging-chain-spec-builder polkadot-omni-node` replaces Nix.

## Check that it answers

```bash
export ROBONOMICS_WS_URL=ws://127.0.0.1:9944
export ROBONOMICS_SURI=//Alice
cps status        # block numbers growing
```

Next: [create nodes and write data](05-managing-nodes.md).
