# Goals

nsh-share does two things. Outbound: hand text, URLs, files and images to the platform's own share sheet — `UIActivityViewController`/`NSSharingService` on Apple, the intent chooser on Android, `DataTransferManager` on Windows, the Web Share API on the web — and report what the user did. Inbound: receive what other apps share into this app, as one cold-start read plus a receive call while the app runs. The iOS share extension and the Android `ACTION_SEND` intent filters are host-app wiring; the crate reads what lands in them.

One facade runs on every platform and is tested through a fake backend on every CI runner. Each bridge compiles in CI, in the job that owns its target: macOS and Windows in the rust job's OS matrix, iOS, Android and web in the cross-check job's targets. No CI runner shows a real sheet or receives a real intent.

## Exactly once

Every call resolves exactly once: a success, a coded error, or a cancel. A cancelled sheet is a result — `ShareResultStatus::Dismissed` — never an error. An answer that arrives after the call has resolved is dropped. Nothing hangs.

## Acceptance criteria

CI is the judge; nothing is accepted on a developer machine. The gates, from `.github/workflows/ci.yml`:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --all-targets`
- in `packages/nsh-share`: `nsh check` and `nsh test`

Passing means: the facade, the gate and the error mapping are proven against the fake backend in both languages, and every bridge compiles for its target. It does not mean a real sheet was shown. Device verification is not claimed anywhere in this repo.

## Error codes

Six stable codes, in `crates/nsh-share/src/error.rs` and `packages/nsh-share/src/errors.ts`. `tests/errors.rs` pins each kind to its code and proves each reaches the caller from a failed start and as the system's answer.

| Code | Trigger | Where it comes from |
|---|---|---|
| `ERR_SHARE_NOT_AVAILABLE` | The target cannot share: the web without the Web Share API, or any target with no bridge | The web bridge maps a missing `navigator.share` to it; `platform/unsupported.rs` answers it for every call and reports `is_available` false |
| `ERR_SHARE_INVALID_ARGUMENT` | The request is malformed and is refused before any sheet starts: no text, no uri and no files; a file path carrying `"://"` or a `"file:"` prefix; on iPad a missing `share_position_origin` | The facade checks the first two on every platform; the iPad anchor check is the Apple bridge's, because presenting without an anchor crashes |
| `ERR_SHARE_IN_PROGRESS` | A second share while a sheet is up, or a second receive while one waits. The first call is untouched and still resolves normally | The facade, every platform |
| `ERR_SHARE_NO_ACTIVITY` | Nowhere to present: no foreground Activity on Android, no window on Windows | The Android and Windows bridges detect it; tests drive it through the fake's `fail_next_share` |
| `ERR_SHARE_FILE_NOT_FOUND` | A listed file path does not exist | The facade stats each path off-wasm before any sheet starts; the backend-side staging step reports the same code (the fake's `fail_next_share` stands in for it) |
| `ERR_SHARE_BACKEND` | The platform call failed with nothing more specific: the sheet invocation, staging or the JNI/objc2 call errored. A platform status code rides along when the platform gives one | Every bridge |

## Edge cases

Each name below is the name of its test. Most exist in both languages: the Rust test in `crates/nsh-share/tests/` and the TypeScript test of the same name in `packages/nsh-share/tests/`. Two are lib unit tests, placed where the platform code they pin compiles: `#[cfg(test)]` inside `crates/nsh-share/src/platform/apple.rs` (which compiles on macOS hosts) and inside `crates/nsh-share/src/platform/unsupported.rs` (which compiles on every target by design).

| Case | What must happen | Test |
|---|---|---|
| `cancel_dismissed_not_error` | Dismissing the sheet resolves `Ok` with `ShareResultStatus::Dismissed`, never an error | `tests/share.rs`; `share.test.ts` |
| `second_share_rejected_in_progress` | While a sheet is up, another share call fails `ERR_SHARE_IN_PROGRESS` immediately, and the first call still resolves normally | `tests/share.rs`; `share.test.ts` |
| `late_answer_ignored_resolve_once` | The first answer resolves the call; a second delivered answer is dropped, and the counters show one resolution | `tests/share.rs`; `share.test.ts` |
| `cancel_share_best_effort` | `cancel_share` reaches the platform seam but never resolves the call; the system's own answer still lands. As with biometrics, the platform keeps the sheet until it answers | `tests/share.rs`; `share.test.ts` |
| `empty_share_invalid_argument` | No text, no uri, no files: `ERR_SHARE_INVALID_ARGUMENT`, no sheet starts | `tests/share.rs`; `share.test.ts` |
| `file_uri_rejected_invalid_argument` | A file path carrying `"://"` or a `"file:"` prefix is rejected with the same code before any sheet starts | `tests/share.rs`; `share.test.ts` |
| `missing_file_file_not_found` | A listed path that does not exist resolves `ERR_SHARE_FILE_NOT_FOUND` before any sheet starts. The facade stats it off-wasm; the fake's `fail_next_share` covers the backend-side staging path | `tests/share.rs`; `share.test.ts` |
| `ipad_anchor_required` | On iPad a missing `share_position_origin` is `ERR_SHARE_INVALID_ARGUMENT`, because presenting without an anchor crashes. Enforced by the Apple bridge, pinned by a `#[cfg(test)]` unit test on a pure helper | `src/platform/apple.rs` (lib unit test) |
| `cold_start_send_vs_main` | `get_initial_media` returns the `SEND` payload and an empty `Vec` for a `MAIN` launch; never an error. Facade tested through the fake; the real intent/activity read is the bridge's, CI-compile-verified | `tests/receive.rs`; `receive.test.ts` |
| `large_file_temp_copy` | A large file is staged to a temp file and shared by a provider URI, never `file://`. The fake-side test asserts `last_params` carries the files and its comment says where the real copy lives: the Android bridge, CI-compile-verified | `tests/share.rs`; `share.test.ts` |
| `no_activity_error` | No foreground Activity (Android) or no window (Windows) answers `ERR_SHARE_NO_ACTIVITY`. Facade tested through `fail_next_share`; the real detection is the bridge's | `tests/share.rs`; `share.test.ts` |
| `web_share_unavailable_not_available` | Web without the Web Share API answers `ERR_SHARE_NOT_AVAILABLE`; a user abort (`AbortError`) answers `Dismissed`. Facade tested through the fake; the mapping is the web bridge's | `tests/share.rs`; `share.test.ts` |
| `platform_unsupported_reports_not_available` | The unsupported target answers `is_available` false and every call `ERR_SHARE_NOT_AVAILABLE` | `src/platform/unsupported.rs` (lib unit test) |
| `in_flight_receive_rejected` | A second receive while one waits answers `ERR_SHARE_IN_PROGRESS`; the waiting call is untouched and the delivered share still resolves it | `tests/receive.rs`; `receive.test.ts` |
| `reset_clears_initial_media` | `reset` empties what `get_initial_media` would return, and never resolves a waiting receive | `tests/receive.rs`; `receive.test.ts` |

`tests/errors.rs` and `errors.test.ts` cover the code table itself: every kind has its code and a message, a platform status is kept with its error, each code reaches the caller from a failed start and as the system's answer, and an `Unavailable` result is an answer, not an error.
