## 🌙 Robonomics Toolbox `@TAG@`

> ⚠️ **Pre-release.** Built from commit [`@SHORT_SHA@`](https://github.com/@REPO@/commit/@SHA@).
> Nightly builds are not audited, not stable and may change without notice.

### ⬇️ Downloads

Every archive contains the `cps`, `mqtt-bridge` and `edge` binaries plus `LICENSE`.

| Platform | Architecture | Download |
| -------- | ------------ | -------- |
| 🐧 Linux (static) | x86_64  | [`robins-@TAG@-linux-x86_64.tar.gz`](@BASE_URL@/robins-@TAG@-linux-x86_64.tar.gz) |
| 🐧 Linux (static) | aarch64 | [`robins-@TAG@-linux-aarch64.tar.gz`](@BASE_URL@/robins-@TAG@-linux-aarch64.tar.gz) |
| 🍎 macOS | Apple Silicon (aarch64) | [`robins-@TAG@-macos-aarch64.tar.gz`](@BASE_URL@/robins-@TAG@-macos-aarch64.tar.gz) |
| 🍎 macOS | Intel (x86_64) | [`robins-@TAG@-macos-x86_64.tar.gz`](@BASE_URL@/robins-@TAG@-macos-x86_64.tar.gz) |

Checksums for all archives: [`SHA256SUMS`](@BASE_URL@/SHA256SUMS).

<details>
<summary>Quick install (Linux / macOS)</summary>

```bash
# pick one of: linux-x86_64, linux-aarch64, macos-aarch64, macos-x86_64
PLATFORM=linux-x86_64
curl -fsSL @BASE_URL@/robins-@TAG@-$PLATFORM.tar.gz | tar -xz
```

Verify a download:

```bash
curl -fsSLO @BASE_URL@/SHA256SUMS
sha256sum --ignore-missing -c SHA256SUMS   # macOS: shasum -a 256 --ignore-missing -c SHA256SUMS
```

Install locally:

```bash
sudo install -m 755 cps mqtt-bridge edge /usr/local/bin/
cps --version
```

</details>

### ❄️ Nix cheatsheet

Nix with [flakes enabled](https://nixos.wiki/wiki/Flakes) is required. Everything below is pinned
to this nightly via the tag; replace `@TAG@` with `master` to follow the development branch.

```bash
# Run without installing
nix run github:@REPO@/@TAG@#libcps -- --help      # cps CLI
nix run github:@REPO@/@TAG@#mqtt-bridge -- --help
nix run github:@REPO@/@TAG@#edge -- --help

# Temporary shell with all tools in PATH
nix shell github:@REPO@/@TAG@#libcps github:@REPO@/@TAG@#mqtt-bridge github:@REPO@/@TAG@#edge

# Install into your profile (and remove)
nix profile install github:@REPO@/@TAG@#libcps
nix profile remove libcps

# Developer shell (Rust toolchain, nextest, taplo, polkadot, ...)
nix develop github:@REPO@/@TAG@

# Use the binary cache to skip compilation
nix run nixpkgs#cachix -- use robonomics
#   ...or answer "y" when nix asks to trust the flake's nixConfig
#   (or pass --accept-flake-config)
```

Packages: `libcps` (binary `cps`), `mqtt-bridge`, `edge` (default).

---

