#!/bin/sh
# Primes the repo's cargo cache with the dependency tree every gate compiles, so the first area gate is incremental, and proves nsh-core 0.4.0 resolves before any area starts.
set -e
ROOT=/Users/hmziq/fo/nsh-share
command -v nsh >/dev/null || { echo "nsh CLI not found" >&2; exit 1; }
nsh --version
PROBE=$(mktemp -d /tmp/nsh-share-warm.XXXXXX)
trap 'rm -rf "$PROBE"' EXIT
mkdir -p "$PROBE/src"
cd "$PROBE"
cat > Cargo.toml <<'EOF'
[package]
name = "nsh-share-warm"
version = "0.0.1"
edition = "2024"
publish = false

[dependencies]
nsh-core = "0.4.0"
thiserror = "2.0.21"
tokio = { version = "1.54.1", default-features = false, features = ["sync", "time"] }
EOF
printf 'pub fn warm() -> u32 { 1 }\n' > src/lib.rs
CARGO_TARGET_DIR="$ROOT/target" cargo check --quiet
CARGO_TARGET_DIR="$ROOT/target" cargo build --quiet
echo "cargo cache warm"
