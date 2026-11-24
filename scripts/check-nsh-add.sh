#!/usr/bin/env bash
# Installs the packaged crate (target/package, from `publish-crates.sh verify`) in a blank app with
# `nsh add` and type-checks an import of it, as `nsh add nsh-share` will once it is on crates.io.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(cargo metadata --no-deps --format-version 1 | jq -er '.packages[] | select(.name == "nsh-share") | .version')
package=$PWD/target/package/nsh-share-$version
[ -d "$package" ] || tar xzf "$package.crate" -C target/package

app=$(mktemp -d)
cd "$app"
echo '{ "name": "consumer", "id": "dev.supernsh.consumer", "entry": "App.tsx" }' > nsh.json
cat > App.tsx <<'TSX'
import { mount, Text } from "nsh";
import * as lib from "nsh-share";

mount(() => <Text>{Object.keys(lib).join(",")}</Text>);
TSX
nsh init .
nsh add --path "$package"
nsh check
nsh build
