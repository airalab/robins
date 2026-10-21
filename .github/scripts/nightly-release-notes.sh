#!/usr/bin/env bash
# Render release notes for a nightly release to stdout.
#
# Usage: nightly-release-notes.sh <tag> <repo> <sha>
#
# The static part (downloads table and Nix cheatsheet) comes from
# .github/release/nightly-notes.md; GitHub-generated changelog is appended
# by the caller.
set -euo pipefail

TAG=${1:?tag is required, e.g. nightly-2610.1}
REPO=${2:?repository is required, e.g. airalab/robins}
SHA=${3:?commit sha is required}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE="${SCRIPT_DIR}/../release/nightly-notes.md"
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"

sed \
    -e "s|@TAG@|${TAG}|g" \
    -e "s|@REPO@|${REPO}|g" \
    -e "s|@SHA@|${SHA}|g" \
    -e "s|@SHORT_SHA@|${SHA:0:7}|g" \
    -e "s|@BASE_URL@|${BASE_URL}|g" \
    "${TEMPLATE}"
