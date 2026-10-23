# Robonomics Toolbox 

[![License](https://img.shields.io/github/license/airalab/robins)](https://github.com/airalab/robins/blob/master/LICENSE)
[![Nightly](https://github.com/airalab/robins/workflows/Nightly/badge.svg)](https://github.com/airalab/robins/actions/workflows/nightly.yml)
[![Downloads](https://img.shields.io/github/downloads/airalab/robins/total.svg)](https://github.com/airalab/robins/releases)

Robonomics Toolbox includes a set of open-source packages for Robotics, Smart Cities and Industry 4.0 developers.

Since Robonomics 5.0 the protocol and the binaries live in separate repositories:

```text
airalab/robonomics        airalab/robins
  runtime                   applications
  pallets                   utilities
  chain specs               service tooling
```

> Download the latest build from the [Releases](https://github.com/airalab/robins/releases).

## Repository Structure

This repository is organized as a Cargo workspace with the following structure:

### Crates

- **`crates/libcps/`** - Robonomics Cyber-Physical System library and CLI application
  - Rust library for managing CPS nodes on-chain
  - CPS tree visualization
  - Robonomics parachain light client
  - Multi-algorithm AEAD encryption (XChaCha20-Poly1305, AES-256-GCM, ChaCha20-Poly1305)
  - Documentation book in [docs/](docs/src/introduction.md) (CLI for devops, library for builders) and [numbered examples](crates/libcps/examples/README.md)
  - See [crates/libcps/README.md](crates/libcps/README.md) for an overview

- **`crates/mqtt-bridge/`** - bridge IoT devices from MQTT to the Robonomics using `libcps`
  - See [crates/mqtt-bridge/README.md](crates/mqtt-bridge/README.md) for detailed documentation

- **`crates/edge/`** - Connectivity Protocol ingress daemon and toolbox for edge devices
  - Single binary: HTTP and Meshtastic ingress, Ed25519 envelope verification, whitelist auth, dedup
  - Republishes accepted messages to a native libp2p GossipSub topic
  - `/health`, `/ready`, `/metrics` and `/debug/peers` out of the box
  - Protocol definitions come in as a git submodule of [`airalab/connectivity-protocol`](https://github.com/airalab/connectivity-protocol) — clone with `--recurse-submodules`
  - See [crates/edge/README.md](crates/edge/README.md) for detailed documentation

### Development Infrastructure

- **`nix/`** - Nix flake modules and build configurations
- **`scripts/`** - Build, deployment, and testing scripts
  - `build-static.sh` - Build statically linked binaries (for docker and portability)

## Contributing

We welcome contributions! Please see our [Contributing Guidelines](./CONTRIBUTING.md).

## Support

- **Wiki**: https://wiki.robonomics.network
- **Issues**: https://github.com/airalab/robins/issues
- **Website**: https://robonomics.network

## License

Robonomics is licensed under the Apache License 2.0. See [LICENSE](./LICENSE) for details.
