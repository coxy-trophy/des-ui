#!/bin/sh
# Strict automatic checks. A blocked integration test is a failed check, not a skip.
set -eu
cd "$(dirname "$0")/.."
for script in ./*.sh scripts/*.sh session/des-ui-session session/des-ui-greeter packaging/*.sh packaging/des-ui-login packaging/des-ui-*/*; do
    [ -f "$script" ] || continue
    sh -n "$script"
done
python3 -m unittest discover -s session/tests -v
python3 -m unittest discover -s packaging/tests -v
python3 -m unittest discover -s companion -v
cargo test --workspace --locked --no-fail-fast
cargo build --workspace --locked
