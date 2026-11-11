# nsh-share

Outbound share of text, URLs, files and images, and inbound share into the app.

Part of the Native Shell (NSH) capability set. Same layout as the other SuperNSH repos: a Rust crate, docs in `docs/`, CI on GitHub Actions, and a web status page on GitHub Pages at https://supernsh.github.io/share/. The outbound names follow Flutter's `share_plus`; the inbound design follows `receive_sharing_intent`. `docs/parity.md` maps every reference API to what it does here.

## Status

**Implemented at the unit-test level.** The behaviors — one sheet at a time, exactly-once resolution, the six stable error codes, the cold-start read — are proven by `cargo test` and `nsh test` in CI, through the fake backend. Each bridge compiles in CI, in the job that owns its target: macOS and Windows in the rust job's OS matrix, iOS, Android and web in the cross-check job's targets. No device or emulator has run this code; nothing here is device-verified. [docs/goals.md](docs/goals.md) lists every behavior and the test that proves it.

## Layout

- `crates/nsh-share/`: the Rust crate — the facade, a backend per platform, the fake backend, `tests/`.
- `packages/nsh-share/`: the TypeScript package, checked by `nsh check` and `nsh test`.
- `docs/`: `brief.md` (the research), `goals.md` (behaviors, errors, edge cases), `parity.md` (the reference APIs), `decisions.md` (choices, versions, licences).
- `site/`: static status page, published by `.github/workflows/pages.yml`.
- `.github/workflows/`: `ci.yml` runs the gates; `pages.yml` publishes the page.

## Checks

Every compile and test runs in GitHub Actions. Nothing is built on a developer machine. CI runs:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings` and `cargo test --all-targets`, the rust job on Linux, macOS and Windows runners — the macOS and Windows bridges compile here
- a cross-check that compiles the crate for `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-linux-android` and `wasm32-unknown-unknown` — the iOS, Android and web bridges compile here
- in `packages/nsh-share`: `nsh check` and `nsh test`

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/share
