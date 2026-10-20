#!/usr/bin/env bash
# Appendix: named profiles keep endpoints and output settings out of your commands.
#
# Profiles never hold secrets: keys stay in ROBONOMICS_SURI or --suri.
set -euo pipefail

CONFIG="$(mktemp -d)/config.toml"
cat > "$CONFIG" <<'TOML'
default-profile = "local"

[profiles.local]
url = "ws://127.0.0.1:9944"
wait = "best"

[profiles.light]
url = "ws://127.0.0.1:9945"
output = "json"
TOML

export CPS_CONFIG="$CONFIG"

# Profiles are resolved without connecting.
cps profile list
cps profile show light

# `--profile` (or CPS_PROFILE) selects a profile; the selected one is shown without NAME.
cps profile show local
cps --profile light profile show

# Against a running light client (`cps lc --listen 127.0.0.1:9945`):
#   cps --profile light status
#   cps --profile light tree 1 --depth 2
