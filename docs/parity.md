# Parity

The names come from three references, read from the clones in `.refs/` on 2026-11-09: `share_plus` 13.3.1 (with `share_plus_platform_interface` 7.2.0) for outbound, `receive_sharing_intent` 1.9.0 for inbound, and `react-native-share` 12.3.1 as a reference only. `docs/decisions.md` says why each was chosen.

Status words:

- **Done**: same name and behavior, proven by a named test.
- **Partial**: works, with the limit named in the row.
- **Accepted**: the field exists here and keeps its default; no bridge reads it.
- **Not ported**: no field or call here, for the reason in the row.
- **Reference only**: a react-native-share item read for names, not ported. Single-target social sheets and UI components are host-app UI, not a capability API.

Rust tests are in `crates/nsh-share/tests/`, lib unit tests beside the platform code they pin, TypeScript tests in `packages/nsh-share/tests/`. A row no test or gate covers says unverified. Each bridge is compile-checked in CI, in the job that owns its target — macOS and Windows in the rust job's OS matrix, iOS, Android and web in the cross-check job; no row claims a device run.

## `share_plus`: entry points

| `share_plus` | Here | Status | Covered by |
|---|---|---|---|
| `SharePlus.instance.share(params)` | `Share::share(&ShareParams)`; `new Share().share(params)` | Done. One params call; a share with no text, no uri and no files is refused, as is a file path that is a URI; one sheet at a time | `empty_share_invalid_argument`; `file_uri_rejected_invalid_argument`; `second_share_rejected_in_progress` |
| `Share.share(text, {subject, sharePositionOrigin})` (deprecated in 13.3.1) | the same call with `text` set | Done. The deprecated Dart shortcut is the params call with fewer fields | `cancel_dismissed_not_error` (a text share through the fake) |
| `Share.shareUri(uri, {sharePositionOrigin})` (deprecated) | `ShareParams.uri`; TS adds `shareUri(uri, title?)` | Done. On iOS the system fetches the page and shows its icon in the sheet — the bridge's part, compile-verified only. macOS, like every other platform, shares the URI as plain text | rides the same call; the iOS fetch is unverified on device |
| `Share.shareXFiles(files, {subject, text, sharePositionOrigin, fileNameOverrides})` (deprecated) | `Share::share_files(files, text, origin)`; `shareFiles(files, text, origin)` | Done. Files with optional text, in the one call | `large_file_temp_copy`; `missing_file_file_not_found` |

`share_plus` also throws `ArgumentError` when `uri` is set together with `text`, on empty text and on a `fileNameOverrides` length mismatch. Here only the nothing-to-share and URI-path refusals exist; the rest is not re-implemented and no test claims it — unverified beyond those two.

## `share_plus`: `ShareParams` fields

| Field | Here | Status | Covered by |
|---|---|---|---|
| `text` | `ShareParams.text` / `text` | Done. The text body; can ride with files | `empty_share_invalid_argument`; `cancel_dismissed_not_error` |
| `title` | `ShareParams.title` / `title` | Partial. Passed to every backend; the Android chooser title and the web share title use it. On Linux it stands in for the subject when none was set — a divergence: `share_plus`'s Linux reads `subject` only. Apple's sheet has no title of its own | `mailto_subject_prefers_subject_then_title` (lib unit test in `src/platform/linux.rs`); "the convenience calls carry what they are given" in `share.test.ts` (title rides `shareUri`) |
| `subject` | `ShareParams.subject` / `subject` | Partial. The mail subject, where the chosen target takes one. On Linux it becomes the mailto subject, with `title` standing in when it is unset — `share_plus`'s Linux reads `subject` only | `mailto_subject_prefers_subject_then_title` (lib unit test in `src/platform/linux.rs`) |
| `uri` | `ShareParams.uri` / `uri` | Done. A URI shares like text, except the iOS page fetch above | counts as content in `empty_share_invalid_argument` |
| `files` | `ShareParams.files: Vec<ShareFile>` / `files` | Partial. Plain filesystem paths; the facade stats existence off-wasm, and the Android bridge turns them into provider URIs. Linux has no provider: files alone are `ERR_SHARE_NOT_AVAILABLE`, and files ride along only when text or a uri carries the share — `share_plus`'s Linux throws `UnimplementedError` whenever files are present | `missing_file_file_not_found`; `large_file_temp_copy`; `mailto_of_files_alone_is_not_available` and `mailto_of_files_with_text_carries_the_text` (lib unit tests in `src/platform/linux.rs`) |
| `fileNameOverrides` | `file_name_overrides` / `fileNameOverrides` | Partial. Same meaning: one name per file. `share_plus` throws on a length mismatch; ours shortens or pads | no named test — unverified beyond the field's presence |
| `sharePositionOrigin` | `share_position_origin: Option<Rect>` / `sharePositionOrigin?: Rect` | Partial. The popover anchor. `share_plus` 13.3.1 documents a center-of-screen fallback on iPad; ours refuses a missing anchor there with `ERR_SHARE_INVALID_ARGUMENT`, because presenting without one crashes | `ipad_anchor_required` (lib unit test in `src/platform/apple.rs`) |
| `previewThumbnail` | `preview_thumbnail` / `previewThumbnail` | Accepted. The field exists in both languages; no bridge reads it. `share_plus` ships it Android-only and marks it a TODO | unverified beyond the field's presence |
| `downloadFallbackEnabled` (default on) | `download_fallback_enabled` / `downloadFallbackEnabled` | Partial. Web only, and only at the files level: when the sheet cannot take the files — `canShare` refuses them, or a path cannot be staged — `true` (the default) downloads each staged file through a clicked hidden anchor and resolves `Unavailable`; `false` answers `ERR_SHARE_NOT_AVAILABLE`. No other platform reads it | the web bridge's part, compile-verified only — no runner exercises the page |
| `mailToFallbackEnabled` (default on) | `mail_to_fallback_enabled` / `mailToFallbackEnabled` | Partial. No effect on web — the bridge ships no mailto fallback there. Linux is the only reader, and the deliberate divergence: mailto is the share there, and `false` answers `ERR_SHARE_NOT_AVAILABLE`, where `share_plus`'s Linux always runs its fallback | `mailto_without_fallback_flag_is_not_available` (lib unit test in `src/platform/linux.rs`); the web no-op is unverified — no runner exercises the page |
| `excludedCupertinoActivities` | no field | Not ported. The Apple sheet shows every activity; filtering `UIActivity` types is objc2-bridge plumbing nothing here drives or tests | none |

## `share_plus`: `ShareResult`

| `share_plus` | Here | Status | Covered by |
|---|---|---|---|
| `ShareResult(raw, status)` | `ShareResult { raw, status }` | Done. `raw` is the platform's own answer string; empty means dismissed, as in `share_plus` | `late_answer_ignored_resolve_once` |
| `ShareResultStatus.success` | `Success` / `"success"` | Done | `status_values_match_share_plus` |
| `ShareResultStatus.dismissed` | `Dismissed` / `"dismissed"` | Done. A dismissal is a result, never an error | `cancel_dismissed_not_error`; the abort half of `web_share_unavailable_not_available` |
| `ShareResultStatus.unavailable` | `Unavailable` / `"unavailable"` | Partial. `share_plus` also uses this status when the environment supports no share results; ours reports that case as `ERR_SHARE_NOT_AVAILABLE` and keeps `Unavailable` for a platform answer that cannot name the action. Linux resolves `Unavailable` at once, the answer `share_plus` reports there: no answer ever comes back from a mail client | `web_share_unavailable_not_available`; `an_unavailable_result_is_an_answer_not_an_error`; the Linux fallback's lib unit tests in `src/platform/linux.rs` |

## `receive_sharing_intent`

| `receive_sharing_intent` | Here | Status | Covered by |
|---|---|---|---|
| `getInitialMedia()` | `ReceiveSharingIntent::get_initial_media() -> Vec<SharedMedia>` / `getInitialMedia()` | Done. The cold-start payload; an empty `Vec` for a `MAIN` launch, never an error. The iOS copy into a temp folder is the bridge's part | `cold_start_send_vs_main` |
| `getMediaStream()` (broadcast stream) | `receive()`: one call waits for the next shared item | Partial. Same design — a cold-start read plus waiting — shaped as one waiter instead of a broadcast stream; an NSH app calls again after each item. A second wait is `ERR_SHARE_IN_PROGRESS` | `in_flight_receive_rejected`; the delivered share in `reset_clears_initial_media` |
| `reset()` | `reset()` | Done. Clears the initial payload, and never resolves a waiting receive | `reset_clears_initial_media` |

### `SharedMediaFile` fields

| Field | Here | Status | Covered by |
|---|---|---|---|
| `path` | `SharedMedia.path` | Done. The path, URL or text, as received | `cold_start_send_vs_main` |
| `thumbnail` | `thumbnail` | Done. The video thumbnail; `None` when the platform gives none | carried by `SharedMedia`; no named test sets it — unverified beyond the field's presence |
| `duration` | `duration` | Done. Video duration in milliseconds | same — unverified beyond the field's presence |
| `type` | `kind: SharedMediaType` | Done. Renamed: `type` is a Rust keyword | `media_kinds_match_receive_sharing_intent` |
| `mimeType` | `mime_type` / `mimeType` | Done | `cold_start_send_vs_main` |
| `message` | `message` | Done. The iOS post message; `None` elsewhere | same — unverified beyond the field's presence |

### `SharedMediaType`

| Value | Here | Status | Covered by |
|---|---|---|---|
| `image`, `video`, `text`, `file`, `url` | `Image`, `Video`, `Text`, `File`, `Url`, each with the same `as_str` value | Done. All five, same strings | `media_kinds_match_receive_sharing_intent` |

## `react-native-share` (reference only)

Read for option names and result shapes. Single-target social sheets and UI components are reference-only: host-app UI, not a capability API.

| `react-native-share` | Here | Status | Covered by |
|---|---|---|---|
| `RNShare.open(options)` | `Share::share(&ShareParams)`; `new Share().share(params)` | Done where it overlaps. `open`'s general sheet is the one call here. RN's `failOnCancel: false` behaviour is the only one here: a dismissal is a result | `cancel_dismissed_not_error` |
| `failOnCancel: true` | no switch | Not ported. A dismissal is always a result; there is no throw-on-cancel mode | `cancel_dismissed_not_error` proves the only mode |
| `shareSingle(options)` | none | Reference only. It targets one named vendor app per call (its `Social` enum) — per-vendor integration, not the platform share capability; an app that wants one target opens its deep link itself | none |
| `isPackageInstalled(packageName)` | none | Reference only. It exists to pick a `shareSingle` target; with no `shareSingle` there is nothing to ask | none |
| `ShareOpenResult { success, message, dismissedAction }` | `ShareResult { status, raw }` | Done where it overlaps. `dismissedAction: true` is `status: "dismissed"`; `success` maps to `Success`; RN's `message` rides `raw` | `status_values_match_share_plus` |
| `ShareSingleOptions` fields | none | Reference only, with `shareSingle` | none |
| `Overlay` | none | Reference only. Host-app UI, not a capability API | none |
| `Sheet` | none | Reference only. Host-app UI, not a capability API | none |
| `Button` | none | Reference only. Host-app UI, not a capability API | none |
| `ShareSheet` | none | Reference only. Host-app UI, not a capability API | none |

### `ShareOptions` fields

| Field | Here | Status | Covered by |
|---|---|---|---|
| `message` | `text` | Done, under `share_plus`'s name | as `text` above |
| `title` | `title` | Done, as `share_plus`'s | as `title` above |
| `url` | `uri` | Done, as `share_plus`'s | as `uri` above |
| `urls` | `uri` or `files` | Partial. RN's `urls` mix web URLs and file paths; ours splits them: one `uri`, or `files` with plain paths | as `uri` and `files` above |
| `type` (MIME hint) | `ShareFile.mime_type` | Partial. Ours carries the MIME per file, not per share | `large_file_temp_copy` asserts `mime_type` |
| `subject` | `subject` | Done, as `share_plus`'s | as `subject` above |
| `email`, `recipient` | none | Not ported. Mail targeting is `shareSingle` territory; a mail share goes through the sheet or a `mailto:` uri | none |
| `excludedActivityTypes` | none | Not ported, as `excludedCupertinoActivities` above | none |
| `showAppsToView` | none | Not ported. Chooser extras are host-app wiring | none |
| `filename`, `filenames` | `file_name_overrides` | Partial, as `fileNameOverrides` above | unverified beyond the field's presence |
| `saveToFiles` | none | Not ported. Saving into the Files app is a different capability from sharing | none |
| `activityItemSources` | none | Not ported. Per-item link metadata for the Apple sheet; nothing here drives it | none |
| `isNewTask` | none | Not ported. Android task flags are the bridge's business | none |
| `disableOverlay` | none | Not ported. RN UI chrome; this library draws nothing | none |
| `useInternalStorage` | none | Not ported. The temp staging location is the bridge's choice | none |
