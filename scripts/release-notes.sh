#!/usr/bin/env bash
#
# Prints the CHANGELOG.md section of one version, as GitHub release notes.
#
#   scripts/release-notes.sh 1.0.0                 # any section
#   scripts/release-notes.sh 1.0.0 --require-date  # only "## [1.0.0] - YYYY-MM-DD"
#
# Relative links are made absolute against the version's tag: a release page
# would resolve them against /releases/ instead of the repository tree.

set -euo pipefail

version="${1:?usage: release-notes.sh <version> [--require-date]}"
require_date="${2:-}"
repo_url="https://github.com/capsulebin/capsulebin"
changelog="$(dirname "$0")/../CHANGELOG.md"

heading="$(grep -E "^## \[${version//./\\.}\]" "${changelog}" || true)"
if [ -z "${heading}" ]; then
  echo "CHANGELOG.md has no section for ${version}" >&2
  exit 1
fi
if [ "${require_date}" = "--require-date" ] && ! grep -qE ' - [0-9]{4}-[0-9]{2}-[0-9]{2}$' <<<"${heading}"; then
  echo "the CHANGELOG.md section for ${version} has no release date: ${heading}" >&2
  exit 1
fi

# The section runs from its heading to the next "## [" heading; link reference
# definitions belong to the whole file and are left out.
awk -v heading="${heading}" '
  $0 == heading { inside = 1; next }
  inside && /^## \[/ { exit }
  inside && /^\[[^]]+\]: / { next }
  inside { print }
' "${changelog}" |
  sed -E "s#\]\(((docs|src|scripts|migrations)/[^)]*|[A-Za-z_-]+\.(md|toml))\)#](${repo_url}/blob/v${version}/\1)#g" |
  sed -e '/./,$!d' -e :a -e '/^\n*$/{$d;N;ba' -e '}'
