#!/usr/bin/env bash
# The `cps` command-line journey: the CLI twin of the library examples 05-07.
#
# Needs a local development node (see docs/src/library/05-launch-local-node.md)
# listening on ws://127.0.0.1:9944 and `cps` in PATH, or run it through cargo:
#
#   cargo build -p libcps && PATH="$PWD/target/debug:$PATH" crates/libcps/examples/shell/cli_journey.sh
set -euo pipefail

export ROBONOMICS_WS_URL="${ROBONOMICS_WS_URL:-ws://127.0.0.1:9944}"
OWNER="${OWNER:-//Alice}"
GUEST="${GUEST:-//Bob}"

# Result of a command as JSON: the id of the created node is `{"node": <id>}`.
node_id() { sed -E 's/.*"node":([0-9]+).*/\1/'; }

echo "== Is the chain reachable?"
cps status

echo "== Create a tree owned by ${OWNER}"
ROOT=$(cps --suri "$OWNER" --output json create --meta '{"type":"building"}' | node_id)
ROOM=$(cps --suri "$OWNER" --output json create --parent "$ROOT" --payload 'temp=22C' | node_id)
cps tree "$ROOT"

echo "== Update data"
cps --suri "$OWNER" payload set "$ROOM" 'temp=24C'
cps payload get "$ROOM"

echo "== ${GUEST} cannot write yet"
GUEST_ACCOUNT=$(cps identity inspect "$GUEST" --output json | sed -E 's/.*"address":"([^"]+)".*/\1/')
cps access check "$ROOM" "$GUEST_ACCOUNT" write || echo "no write access (exit $?)"

echo "== Grant ${GUEST} write access to the room and let them write"
cps --suri "$OWNER" access grant "$ROOM" "$GUEST_ACCOUNT" write
cps --suri "$GUEST" payload set "$ROOM" 'temp=25C (written by guest)'
cps payload get "$ROOM"

echo "== Encrypt a value for ${GUEST} and decrypt it"
cps --suri "$OWNER" payload set "$ROOM" 'secret reading' --receiver-public "$GUEST_ACCOUNT"
cps payload get "$ROOM" --hex
cps --suri "$GUEST" payload get "$ROOM" --decrypt

echo "== Clean up"
cps --suri "$OWNER" delete --yes "$ROOM"
cps --suri "$OWNER" delete --yes "$ROOT"
