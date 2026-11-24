#!/usr/bin/env bash
# Enforces the Waylingo rule "provider data never reaches an LLM" at the crate level:
# the LLM crate (waylingo-parse) and the provider crate (waylingo-providers) must not
# depend on each other in either direction.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
check() { # $1 = crate, $2 = forbidden dependency
  if cargo metadata --format-version 1 --no-deps >/dev/null 2>&1 \
     && cargo tree -p "$1" --edges normal,build --prefix none 2>/dev/null | grep -q "^$2 "; then
    echo "FORBIDDEN: $1 depends on $2"; fail=1
  fi
}
check waylingo-parse waylingo-providers
if cargo tree -p waylingo-providers >/dev/null 2>&1; then
  check waylingo-providers waylingo-parse
fi
[ "$fail" -eq 0 ] && echo "crate boundaries ok"
exit "$fail"
