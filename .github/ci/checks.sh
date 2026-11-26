#!/usr/bin/env bash
set -euo pipefail

# The shared workspace test covers all features together. Each forge must also
# compile on its own, and the package must build without the excluded tests.
mbx check --locked --no-default-features
for feature in github gitlab tea; do
  mbx check --locked --no-default-features --features "$feature"
done
cargo publish --dry-run --locked
