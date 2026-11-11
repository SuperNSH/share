# Decisions

Each entry says what was chosen and why. `docs/goals.md` lists the behaviors and their tests; `docs/parity.md` lists what each reference API does here.

## Decided

1. **NSH stays on 0.4.0.** The installed CLI reports `nsh 0.4.0 (e626a8d)`, and the published `nsh-core` the crate resolves is also 0.4.0 (`nsh-core = "0.4.0"` in `crates/nsh-share/Cargo.toml`). Nothing share needs exists only on nsh's unreleased `main`: the facade needs the core's runtime and test driver, and the TS package needs `nsh check` and `nsh test`, all of which 0.4.0 ships. Staying on the release means CI and every developer run the same version.

2. **Libraries: port `share_plus` for outbound, `receive_sharing_intent`'s design for inbound, `react-native-share` reference only.** Outbound ports `share_plus` 13.3.1's shape — one `ShareParams` call, a `ShareResult` with a status — because the platform contracts it encodes are the ones worth copying: `UIActivityViewController`/`NSSharingService` through objc2 on Apple, the intent chooser through JNI on Android, `DataTransferManager` on Windows, the Web Share API on the web, and the honest `mailto:`/`xdg-open` fallback on Linux. The port takes names and behavior, not code. Inbound ports `receive_sharing_intent` 1.9.0's design — a cold-start read plus a listen — because cold start is the hard part: a share that launches the app delivers its payload before the app's code runs, and a `MAIN` launch must read as empty, never an error. It is reshaped as one waiter at a time, which fits an NSH app's re-call loop. `react-native-share` is reference only: its single-target social sheets and its UI components are host-app UI, not a capability API.

3. **Platforms: outbound on iOS, macOS, Android, Windows, Linux and web; inbound on iOS and Android.** Every platform in the outbound list has a system sheet or the Web Share API; a target with none gets the unsupported module, which answers `is_available` false and every call `ERR_SHARE_NOT_AVAILABLE`. Inbound exists only where an OS delivers shares into an app: the iOS share extension and Android's `ACTION_SEND` intent filters. Desktop and web apps receive nothing from other apps, so the Linux backend answers `can_receive` false and refuses a listen rather than hanging. The extension declaration and the intent filters are host-app wiring — the app's Xcode target and manifest — which a crate cannot write; the crate reads what lands in them.

## Open

None.

## Versions and licences

Read from the cloned repos' own manifests and LICENSE files on 2026-11-09. No registry or network was consulted.

| Reference | Version | Licence |
|---|---|---|
| `share_plus` | 13.3.1 (`packages/share_plus/share_plus/pubspec.yaml`) | BSD 3-Clause |
| `share_plus_platform_interface` | 7.2.0 (`pubspec.yaml`) | BSD 3-Clause |
| `receive_sharing_intent` | 1.9.0 (`pubspec.yaml`) | Apache-2.0 |
| `react-native-share` | 12.3.1 (`package.json`) | MIT |
| `nsh` (main branch) | workspace 0.4.0 (`Cargo.toml`); the installed CLI is `nsh 0.4.0 (e626a8d)` | Apache-2.0 |
| `biometrics` (sibling repo; house style for these docs) | — | MIT |

All five reference URLs in `.zcode/step0-refs.sh` cloned successfully on 2026-11-09 — `fluttercommunity/plus_plugins`, `KasemJaffer/receive_sharing_intent`, `react-native-share/react-native-share`, `longbridge/nsh`, `SuperNSH/biometrics` — so no canonical-repo substitution was needed.
