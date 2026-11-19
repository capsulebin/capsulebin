#!/usr/bin/env bash
#
# Builds the web client twice from scratch and fails unless both builds are
# byte for byte identical.
#
#   scripts/reproducible-web.sh
#
# The release binary embeds src/server/web/dist, so anyone rebuilding a tagged
# version has to arrive at the same assets to check that a server ships the
# published client. Leaves the second build in src/server/web/dist.

set -euo pipefail

cd "$(dirname "$0")/.."

manifest() {
  (cd src/server/web/dist && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum)
}

build() {
  rm -rf -- src/server/web/dist
  npm run build -w @capsulebin/web >/dev/null
}

build
first="$(manifest)"
build
second="$(manifest)"

if [ "${first}" != "${second}" ]; then
  echo "the web build is not reproducible; files that differ between two builds:" >&2
  diff <(printf '%s\n' "${first}") <(printf '%s\n' "${second}") >&2 || true
  exit 1
fi

printf '%s\n' "${second}"
echo "ok  two web builds are identical ($(printf '%s\n' "${second}" | wc -l) files)"
