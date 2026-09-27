[<img align="right" src="https://github.com/airalab/robonomics/blob/master/web3_foundation_grants_badge_black.jpg">](https://medium.com/web3foundation/web3-foundation-grants-wave-two-recipients-16d9b996501d)

# Robonomics Binaries

[![License](https://img.shields.io/github/license/airalab/robins)](https://github.com/airalab/robins/blob/master/LICENSE)
[![Release](https://img.shields.io/github/release/airalab/robins.svg)](https://github.com/airalab/robins/releases)
[![Nightly](https://github.com/airalab/robins/workflows/Nightly/badge.svg)](https://github.com/airalab/robins/actions/workflows/nightly.yml)
[![Downloads](https://img.shields.io/github/downloads/airalab/robins/total.svg)](https://github.com/airalab/robins/releases)

> Robonomics node and tools built in Rust on top of the [Polkadot SDK](https://polkadot.com/platform/sdk/). For more specific guides see the [Robonomics Wiki](https://wiki.robonomics.network).

Robonomics platform includes a set of open-source packages and infrastructure for Robotics, Smart Cities and Industry 4.0 developers.

Since Robonomics 5.0 the protocol and the binaries live in separate repositories:

```text
airalab/robonomics        airalab/robins
  runtime                   blockchain node
  pallets                   utilities
  chain specs               service tooling
```

The runtime, the pallets and the chain specs are consumed from [crates.io](https://crates.io/crates/robonomics-runtime) (`robonomics-runtime`, `robonomics-runtime-subxt-api`, `robonomics-chain-spec`), so nothing here vendors or checks out the protocol repository.

## Repository Structure

This repository is organized as a Cargo workspace with the following structure:

### Crates

- **`crates/robonomics/`** - the Robonomics Omni Node
  - Built on `polkadot-omni-node-lib` for maximum Polkadot compatibility
  - Chain specs come from the `robonomics-chain-spec` crate — there is no `chains/` directory here

- **`crates/libcps/`** - Robonomics CPS (Cyber-Physical Systems) library and CLI
  - Rust library for managing hierarchical CPS nodes on-chain
  - CLI with colored output and tree visualization
  - Multi-algorithm AEAD encryption support (XChaCha20-Poly1305, AES-256-GCM, ChaCha20-Poly1305)
  - See [crates/libcps/README.md](crates/libcps/README.md) for detailed documentation

- **`crates/mqtt-bridge/`** - bridge IoT devices from MQTT to the Robonomics Network
  - Split out of `libcps` so that the CPS library does not carry an MQTT client
  - See [crates/mqtt-bridge/README.md](crates/mqtt-bridge/README.md) for detailed documentation

- **`crates/edge/`** - Connectivity Protocol ingress daemon and CLI toolbox for edge devices
  - Single binary: HTTP and Meshtastic ingress, Ed25519 envelope verification, whitelist auth, dedup
  - Republishes accepted messages to a native libp2p GossipSub topic
  - `/health`, `/ready`, `/metrics` and `/debug/peers` out of the box
  - Protocol definitions come in as a git submodule of [`airalab/connectivity-protocol`](https://github.com/airalab/connectivity-protocol) — clone with `--recurse-submodules`
  - See [crates/edge/README.md](crates/edge/README.md) for detailed documentation

### Development Infrastructure

- **`nix/`** - Nix flake modules and build configurations
- **`scripts/`** - Build, deployment, and testing scripts
  - `build-deb.sh` - Debian package builder
  - `build-static.sh` - Build statically linked binaries (for docker and portability)
  - `try-runtime.sh` - Runtime upgrade checks
  - `docker/` - Docker runner image and healthcheck scripts
  - `docker/builder` - Docker builder image

## Contributing

We welcome contributions! Please see our [Contributing Guidelines](./CONTRIBUTING.md).

## Support

- **Wiki**: https://wiki.robonomics.network
- **Issues**: https://github.com/airalab/robins/issues
- **Website**: https://robonomics.network

## License

Robonomics is licensed under the Apache License 2.0. See [LICENSE](./LICENSE) for details.
