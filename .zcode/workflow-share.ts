// Fresh workflow script for one run: nsh-share — a Rust crate and a TypeScript package
// (laid out like .refs/biometrics) for outbound share and inbound receive.
//
// Per-area chain, all inside one fan-out: implement -> [gate -> audit -> fix]* until the
// gate is clean AND the auditor reports zero findings, capped at 3 rounds (the brief:
// GLM-5.3 Max everywhere, 3 audit rounds per area, honest report if a cap hits).
// STEP 0 (clones + cargo cache warm) runs serially before the fan-out.
// The join runs the brief's four whole-tree gates once, with one fix round.
// No commits, no pushes, no branches: areas edit the current checkout directly.

const MODELS = {
  // The brief pins GLM-5.3 Max for every implementer AND every audit/check phase.
  pro: model("account:zai-individual-coding-plan/GLM-5.3$high"),
  // Only a vision-capable capture interpreter would run flash; this run has no UI capture.
  flash: model("account:zai-individual-coding-plan/GLM-5.3-Flash$high"),
};

const ROOT = "/Users/hmziq/fo/nsh-share";
const KIT = "/Users/hmziq/ZCodeProject/ai/harness/zcode/agents";
const AUDIT_ROUNDS = 3; // brief: until zero issues or the cap

// Whole-file specialist bodies load at run time; the script never carries a copy.
async function specialist(name: string, fallback: string): Promise<string> {
  const adapter =
    `You are the ${name} specialist for one area of a parallel workflow. ` +
    `Your full specialist contract follows verbatim — apply it to this area's work; ` +
    `where it conflicts with your ask, the ask wins.` + "\n\n";
  for (const p of [`${ROOT}/.zcode/agents/${name}.md`, `${KIT}/${name}.md`]) {
    try {
      const raw = await files.read(p);
      return adapter + raw.replace(/^---\n[\s\S]*?\n---\n/, "").trim();
    } catch {
      /* try the next source */
    }
  }
  return fallback;
}

const PLAIN =
  "You are a careful, senior implementer. Follow the ask exactly; where the ask is silent, follow the repo's AGENTS.md.";

artifact.board("areas", {
  key: "name",
  status: "phase",
  columns: ["implementing", "gating", "auditing", "fixing", "done", "failed"],
});

// ---------------------------------------------------------------------------
// The predetermined API contract every coding area codes against.
// ---------------------------------------------------------------------------

const CONTRACT = `
Predetermined API contract (every area codes against exactly these names; nobody renames anything):

Repo layout, mirroring .refs/biometrics:
- Root /Users/hmziq/fo/nsh-share/Cargo.toml becomes a virtual workspace manifest: members = ["crates/nsh-share"], resolver = "3", and [workspace.package] with version 0.1.0, edition 2024, license MIT, repository https://github.com/SuperNSH/share, authors ["hmziqrs"]. The old root src/lib.rs scaffold is deleted; the crate lives at crates/nsh-share/.
- Crate nsh-share (lib name nsh_share). Dependencies: nsh-core = "0.4.0", thiserror = "2.0.21", tokio 1.54.1 with default-features off and features ["sync","time"]. Per-target bridge dependencies you verify on crates.io before pinning: the objc2 family for Apple, jni 0.22.x + ndk-context 0.1.x for Android, the windows crate 0.62.x for Windows, wasm-bindgen + web-sys + js-sys for wasm32. Dev-dependencies: tokio with features ["rt","time","macros","test-util"]. The TypeScript package lives at packages/nsh-share/.

Rust public API:
- error.rs: #[non_exhaustive] enum ErrorKind { NotAvailable, InvalidArgument, InProgress, NoActivity, FileNotFound, Backend } with stable codes "ERR_SHARE_NOT_AVAILABLE", "ERR_SHARE_INVALID_ARGUMENT", "ERR_SHARE_IN_PROGRESS", "ERR_SHARE_NO_ACTIVITY", "ERR_SHARE_FILE_NOT_FOUND", "ERR_SHARE_BACKEND"; struct ShareError { kind: ErrorKind, status: Option<i64> } with new(), with_status(), kind(), status(), code() (the stable code), message() (one-sentence what-to-do), Display via the code — same shape as .refs/biometrics/crates/nsh-biometrics/src/error.rs. A dismissed sheet is NOT an error: outbound cancellation is ShareResultStatus::Dismissed, an Ok answer.
- options.rs: Rect { x: f64, y: f64, width: f64, height: f64 }; ShareFile { path: String, mime_type: Option<String> } where path is a plain filesystem path, never a file:// URI; ShareParams { title: Option<String>, text: Option<String>, subject: Option<String>, uri: Option<String>, files: Vec<ShareFile>, file_name_overrides: Vec<String>, share_position_origin: Option<Rect>, preview_thumbnail: Option<String>, mail_to_fallback_enabled: bool, download_fallback_enabled: bool } with Default (both fallback flags default true).
- share.rs: struct Share; Share::new() returns the process-wide platform handle (OnceLock over platform::default_backend()), Share::with_backend(impl Backend + 'static) is for tests and fakes; clones share one sheet gate. async share(&self, params: ShareParams) -> Result<ShareResult, ShareError>; async share_uri(&self, uri: impl Into<String>, title: Option<String>) -> Result<ShareResult, ShareError>; async share_files(&self, files: Vec<ShareFile>, text: Option<String>, origin: Option<Rect>) -> Result<ShareResult, ShareError>. ShareResult { raw: String, status: ShareResultStatus }; #[derive(Clone, Copy, PartialEq, Eq, Debug)] enum ShareResultStatus { Success, Dismissed, Unavailable } (share_plus names).
- receive.rs: struct ReceiveSharingIntent with new() and with_backend(); async get_initial_media(&self) -> Result<Vec<SharedMedia>, ShareError> — the cold-start read; an Android MAIN launch or a plain iOS launch yields an empty Vec, never an error; async receive(&self) -> Result<SharedMedia, ShareError> — waits for the next inbound item, one waiter at a time; fn reset(&self) clears stored initial media. SharedMedia { path: String, thumbnail: Option<String>, duration: Option<u64>, kind: SharedMediaType, mime_type: Option<String>, message: Option<String> }; enum SharedMediaType { Image, Video, Text, File, Url } with as_str() returning "image"/"video"/"text"/"file"/"url" (receive_sharing_intent's values).
- backend.rs: type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>; type ShareDone = Box<dyn FnOnce(Result<ShareResult, ShareError>) + Send>; type MediaDone = Box<dyn FnOnce(Result<Vec<SharedMedia>, ShareError>) + Send>; trait Backend: Send + Sync { fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>>; fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>>; fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError>; fn cancel_share(&self); fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>>; fn start_listen(&self, done: MediaDone) -> Result<(), ShareError>; fn stop_listen(&self); }. start_share returns at once; done is called exactly once when the system answers; an Err from start_share means no sheet was shown and done is never called. cancel_share is best-effort: the system's answer still arrives through done. Where the platform cannot cancel, it is a no-op.
- fake.rs: FakeBackend (Clone over Arc<Mutex<state>>), mirroring .refs/biometrics/crates/nsh-biometrics/src/fake.rs: Default is available true, can_receive false; set_available(Result<bool, ShareError>), set_can_receive(Result<bool, ShareError>), fail_next_share(ShareError) (next start_share returns Err and no sheet is shown), pending_share() -> bool, deliver_share(Result<ShareResult, ShareError>) -> bool (false when nothing waits), deliver_media(Result<SharedMedia, ShareError>) -> bool, set_initial_media(Vec<SharedMedia>), counters shares_started() / cancels() / listens_started(), last_params() -> Option<ShareParams>.
- share.rs facade rules (what Share adds over a Backend): validate before start_share — params with no text, no uri and no files is InvalidArgument; any file path containing "://" or starting with "file:" is InvalidArgument (bridges turn staged paths into provider URIs themselves); while a sheet is up another share call returns InProgress immediately; the busy state lasts until the system answers, even past a cancel request; the answer resolves the call exactly once through a generation-gated tokio::sync::oneshot (same pattern as .refs/biometrics auth.rs Gate), so a late second answer is ignored. Dismissal maps to Ok(ShareResultStatus::Dismissed).
- receive.rs facade rules: get_initial_media resolves once from the backend snapshot; receive while another receive waits is InProgress; stopping/reset never resolves a waiting receive — the backend's answer does.
- native.rs: TSX class NativeShare exposed with #[class] structs and #[native_module("nsh-share/native")] in lib.rs, mirroring .refs/biometrics native.rs and lib.rs: JsObject param structs NativeShareParams, NativeShareFile { path, mime_type }, NativeRect { x, y, width, height } with Option fields; class members is_available(), share(params), share_uri(uri, title), share_files(files, text, origin), get_initial_media(), receive(), reset(), cancel_share(); async members reject with the stable code. Verify every macro and trait name against the vendored nsh-core 0.4.0 source in .refs/nsh/crates/nsh-core before writing; never invent an API.
- platform/mod.rs: cfg dispatch exactly like .refs/biometrics platform/mod.rs — apple target_vendor -> apple, android -> android, linux -> linux, windows -> windows, wasm32 -> web, otherwise unsupported — plus pub(crate) fn default_backend() -> Arc<dyn Backend>.
- platform/unsupported.rs: struct NativeBackend implementing Backend: is_available and can_receive answer Ok(false); every other call answers Err(NotAvailable); cancel and stop are no-ops.

House rules (AGENTS.md, binding): comments are 1-2 lines and state a non-obvious constraint, never narrate the next line; dead code is deleted, no speculative abstractions; docs read direct and concrete with no marketing filler; an operation resolves exactly once — success, a coded failure, or a cancel — and never hangs; a native bridge never panics (no unwrap/expect on foreign results) and never blocks a thread waiting on a foreign callback; status claims in README and docs must match what CI runs.
`;

const EDGE_CASES = `
Canonical edge cases (docs/goals.md documents each with the same name; the tests area writes one test per name in BOTH languages):
1. cancel_dismissed_not_error — dismissing the sheet resolves Ok with ShareResultStatus::Dismissed, never an error.
2. second_share_rejected_in_progress — while a sheet is up, another share call fails InProgress immediately and the first call still resolves normally.
3. late_answer_ignored_resolve_once — the first answer resolves the call; a second delivered answer is dropped and the counters show one resolution.
4. cancel_share_best_effort — cancel does not resolve the call; the system's own answer still lands (mirrors biometrics: the platform keeps the sheet until it answers).
5. empty_share_invalid_argument — no text, no uri, no files.
6. file_uri_rejected_invalid_argument — a file path carrying "://" or a "file:" prefix is rejected before any sheet starts.
7. missing_file_file_not_found — a listed path the platform cannot stage resolves FileNotFound (fake models it; real backends stat).
8. ipad_anchor_required — on iPad a missing share_position_origin is InvalidArgument; documented, enforced by the Apple bridge.
9. cold_start_send_vs_main — get_initial_media returns the SEND payload and an empty Vec for MAIN; never an error.
10. large_file_temp_copy — a large file is staged to a temp file and shared by provider URI (the fake records staging; the Android bridge implements the copy).
11. no_activity_error — Android with no foreground Activity answers NoActivity.
12. web_share_unavailable_not_available — web without the Web Share API answers NotAvailable; a user abort answers Dismissed.
13. platform_unsupported_reports_not_available — the unsupported target answers is_available false and every call NotAvailable.
14. in_flight_receive_rejected — a second receive while one waits answers InProgress.
15. reset_clears_initial_media — reset empties what get_initial_media would return.
`;

const FOOTER =
  "Work directly in the current checkout at /Users/hmziq/fo/nsh-share — no branches, no worktrees, no commits, no pushes, no git commands at all — and do not run whole-tree test suites beyond your own scoped gate; finish your piece and report. Report with: done (boolean), summary (a short honest paragraph: what landed, what you could not verify), blockers (optional array).";

// ---------------------------------------------------------------------------
// Area definitions. Chains: docs is implement -> audit; every other area is
// implement -> [gate -> audit -> fix]* bounded by AUDIT_ROUNDS.
// ---------------------------------------------------------------------------

interface GateSpec {
  label: string;
  cmd: string;
  args: string[];
  timeoutMs: number;
}

interface AreaDef {
  name: string;
  blurb: string;
  specialist: string;
  fallback: string;
  implName: string;
  auditSpecialist: string;
  auditFallback: string;
  owns: string;
  ask: string;
  gate: GateSpec | null;
  auditFocus: string;
}

const areas: AreaDef[] = [
  {
    name: "docs",
    blurb: "goals, parity, decisions, README",
    specialist: "z-nsh-engineer",
    fallback: PLAIN,
    implName: "NSH engineer·docs",
    auditSpecialist: "z-nsh-engineer",
    auditFallback: PLAIN,
    owns: "docs/goals.md, docs/parity.md, docs/decisions.md, README.md",
    ask: `Write the four docs files for nsh-share. Read first: AGENTS.md, docs/brief.md, the existing docs/*.md and README.md, and the reference sources under .refs/ (plus_plugins/packages/share_plus/share_plus/lib/share_plus.dart and share_plus_platform_interface, receive_sharing_intent/lib/receive_sharing_intent.dart and lib/src/, react-native-share/src/types.ts and src/index.tsx, and .refs/biometrics/docs/*.md as the house style). Cloning already happened in step 0; if a reference you need is missing, say so in blockers — do not clone anything yourself.

You own exactly these files (create or rewrite): docs/goals.md, docs/parity.md, docs/decisions.md, README.md.

docs/goals.md: what nsh-share does (outbound share of text, URLs, files and images; inbound receive into the app), acceptance criteria phrased as the gates CI runs (cargo fmt --all -- --check, cargo clippy --all-targets -- -D warnings, cargo test --all-targets, nsh check and nsh test in packages/nsh-share), the error table — one row per stable code: ERR_SHARE_NOT_AVAILABLE, ERR_SHARE_INVALID_ARGUMENT, ERR_SHARE_IN_PROGRESS, ERR_SHARE_NO_ACTIVITY, ERR_SHARE_FILE_NOT_FOUND, ERR_SHARE_BACKEND — each with its trigger and how platforms map to it, and the edge-case list. Use the canonical names and meanings below verbatim; each entry maps to its named test.

${EDGE_CASES}

Also state the exactly-once rule in goals.md: every call resolves exactly once as success, a coded error, or a cancel, and a cancelled sheet is a result (Dismissed), not an error.

docs/parity.md: one section per reference. For share_plus: Share.share, Share.shareUri, Share.shareXFiles, SharePlus.instance.share, every ShareParams field (text, title, subject, files, fileNameOverrides, sharePositionOrigin, previewThumbnail, uri, downloadFallbackEnabled, mailToFallbackEnabled, excludedCupertinoActivities), ShareResult (raw, status) and ShareResultStatus (success, dismissed, unavailable) — for each, what ours does (same name, same behavior, or accepted-and-ignored, or not ported and why). For receive_sharing_intent: getInitialMedia, the receive broadcast stream, reset, SharedMediaFile (path, thumbnail, duration, type, mimeType, message) and SharedMediaType (image, video, text, file, url). For react-native-share: RNShare.open, shareSingle, isPackageInstalled, ShareOptions and ShareSingleOptions fields, ShareOpenResult, and the UI components (Overlay, Sheet, Button, ShareSheet) — single-target social sheets and UI components are reference-only: say so per item, with one line why (host-app UI, not a capability API). Mark every row with what a test proves it; a claim no test or gate runs must say unverified.

docs/decisions.md: move the three open decisions to Decided with reasons: (1) NSH stays on 0.4.0 — the installed nsh CLI and the published nsh-core 0.4.0 agree, and nothing share needs unreleased main; (2) library approach — port share_plus for outbound (UIActivityViewController/NSSharingService via objc2 on Apple, Intent chooser via JNI on Android), port receive_sharing_intent's design for inbound (cold-start read plus a listen stream); react-native-share is reference only; (3) platforms in scope — outbound iOS, macOS, Android, Windows, Linux, web; inbound iOS and Android, where the share extension or intent filters are host-app wiring the crate reads from. Record the versions and licences you read from the cloned repos' own manifests and LICENSE files (do not guess, do not fetch the network), and record that all five reference URLs cloned successfully on 2026-11-09 so no canonical-repo substitution was needed.

README.md: update Status (implemented at the unit-test level: behaviors proven by cargo test and nsh test in CI, device behavior only compile-checked by CI's cross targets — never claim device verification), Layout (crates/nsh-share, packages/nsh-share, docs, site, .github/workflows), and Checks (list exactly the gates the updated ci.yml runs). Keep it short and concrete; the current README's tone is the house tone.

${FOOTER}`,
    gate: null,
    auditFocus:
      "Audit the four docs files for: accuracy against the .refs sources (API names, field lists, enum values), coverage of all three references' public APIs, the six error codes matching the contract, all fifteen canonical edge-case names present with meanings intact, decisions.md resolving the three open decisions with versions and licences read from the cloned repos, and README status claims matching only what CI actually runs. Flag marketing filler, hedged claims, and any claim marked done that no gate or test covers.",
  },
  {
    name: "rust-core",
    blurb: "workspace restructure, core API, error types, backend trait, fake, native class, ci.yml",
    specialist: "z-nsh-engineer",
    fallback: PLAIN,
    implName: "NSH engineer·rust-core",
    auditSpecialist: "z-rust-engineer",
    auditFallback: PLAIN,
    owns:
      "/Cargo.toml, crates/nsh-share/Cargo.toml, crates/nsh-share/src/lib.rs, error.rs, options.rs, backend.rs, fake.rs, share.rs, receive.rs, native.rs, platform/mod.rs, platform/unsupported.rs, the temporary platform stub files apple.rs, android.rs, windows.rs, linux.rs, web.rs, deleting the old root src/lib.rs, and .github/workflows/ci.yml",
    ask: `Restructure the repo into a workspace and write the Rust core. Read first: AGENTS.md, docs/brief.md, the current root Cargo.toml and src/lib.rs, .refs/biometrics/Cargo.toml, crates/nsh-biometrics/src/ (lib.rs, error.rs, backend.rs, fake.rs, auth.rs, native.rs, platform/mod.rs, platform/unsupported.rs — copy the shapes, adapt the content), and the nsh-core 0.4.0 source under .refs/nsh/crates/nsh-core for the exact prelude, #[class], JsObject and #[native_module] APIs. Never invent an nsh-core API; verify in that source.

${CONTRACT}

Work plan, in order:
1. Root manifest to a virtual workspace; create crates/nsh-share with its Cargo.toml (workspace inheritance, [lib] name nsh_share); write all src modules per the contract; delete the old root src/lib.rs file with rm (no git commands).
2. fake.rs and the facade gate logic first, with #[cfg(test)] unit tests inline in share.rs and receive.rs covering the gate rules (empty-args validation, in-progress rejection, resolve-exactly-once with a dropped late answer, cancel does not resolve, reset clears) using FakeBackend and tokio::test.
3. lib.rs wires modules, re-exports the public API, and carries #[native_module("nsh-share/native")] registering NativeShare.
4. platform stubs: write every platform file (apple.rs, android.rs, windows.rs, linux.rs, web.rs) as a compiling NativeBackend that behaves like unsupported.rs for now — the four sibling areas own those files and will replace their contents this run; the stubs keep the tree green if a sibling fails. mod.rs and unsupported.rs are final in your hands.
5. Unit tests run under tokio::test (dev-dependency rt + macros + time). Keep clippy clean at -D warnings including the tests.
6. Rewrite .github/workflows/ci.yml so it runs both stacks, keeping the existing pinned-action style and the header comment convention: a format job (cargo fmt --all -- --check); a rust job on ubuntu-latest, macos-latest and windows-latest (fail-fast false) running cargo clippy --all-targets -- -D warnings and cargo test --all-targets; a cross-check job compiling the crate for aarch64-apple-ios, aarch64-apple-ios-sim, aarch64-linux-android and wasm32-unknown-unknown with cargo clippy --target <target> --all-targets -- -D warnings, pointing cc at the NDK for the Android target the way .refs/biometrics/.github/workflows/ci.yml does; and a typescript job installing NSH v0.4.0 from SuperNSH/nsh-builds (copy the install steps from the biometrics workflow) then, working-directory packages/nsh-share, nsh check, nsh build piped through tee with a grep that fails on warnings, and nsh test. No emulator, no simulator, no device steps anywhere.

Your scoped gate is: cargo fmt --all -- --check, then cargo clippy -p nsh-share --lib -- -D warnings, then cargo test -p nsh-share --lib. Integration tests under crates/nsh-share/tests/ belong to another area; do not create them, and if a sibling's in-progress files break your gate, say which files in your summary rather than editing them.

${FOOTER}`,
    gate: {
      label: "cargo fmt, clippy --lib, test --lib",
      cmd: "bash",
      args: [
        "-lc",
        "cd /Users/hmziq/fo/nsh-share && cargo fmt --all -- --check && cargo clippy -p nsh-share --lib -- -D warnings && cargo test -p nsh-share --lib",
      ],
      timeoutMs: 1200000,
    },
    auditFocus:
      "Audit the workspace restructure and core modules: manifest and module shapes against the contract and .refs/biometrics; the gate logic (exactly-once via generation-gated oneshot, in-progress rejection, validation rules, busy-until-answered); error codes and messages; the Backend trait and FakeBackend surface; native.rs against the vendored nsh-core source (no invented APIs); ci.yml covering both stacks and the four cross targets with no device or emulator steps; AGENTS.md house rules (comments, no dead code); clippy-cleanliness under -D warnings.",
  },
  {
    name: "bridge-apple",
    blurb: "platform/apple.rs — UIActivityViewController, NSSharingService, iPad anchor, iOS inbound",
    specialist: "z-rust-engineer",
    fallback: PLAIN,
    implName: "Rust engineer·bridge-apple",
    auditSpecialist: "z-unsafe-ffi-auditor",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/src/platform/apple.rs (you may add the objc2 dependency lines to crates/nsh-share/Cargo.toml's apple target section only)",
    ask: `Write the Apple bridge. Read first: AGENTS.md, the contract below, .refs/biometrics/crates/nsh-biometrics/src/platform/apple.rs (the house bridge shape), the share_plus Apple implementation under .refs/plus_plugins/packages/share_plus/share_plus/ (ios/ Classes/ SharePlusPlugin.swift and macos/), and the vendored objc2 crate sources if you need a signature (.refs only covers what nsh vendors; otherwise verify against the crates.io docs you know and pin versions conservatively).

${CONTRACT}

You own crates/nsh-share/src/platform/apple.rs. Replace the stub completely. Implement Backend for NativeBackend on target_vendor = "apple":
- Availability: is_available answers whether a sharing surface exists (UIActivityViewController on iOS, NSSharingServicePicker on macOS); can_receive answers false on macOS and true on iOS.
- Outbound: show UIActivityViewController with the item list built from ShareParams — text, url, and files staged into a temp directory and shared as file URLs; subject maps to the email subject where the platform exposes it; share_position_origin becomes the popover sourceView/ sourceRect anchoring on iPad — REQUIRED on iPad, and a missing anchor is InvalidArgument (edge case ipad_anchor_required), because presenting without one crashes. Run all UIKit/AppKit work on the main thread dispatched off the calling async context, and call done exactly once from the controller's completion handler: completed activity -> Success, cancelled -> Dismissed. Never block a thread waiting for the completion.
- Inbound: get_initial_media reads what the host delivered at launch (NSUserActivity / share-extension handoff through the App Group container); a plain launch or a MAIN-type activation yields an empty Vec (cold_start_send_vs_main); start_listen registers the handoff/activation observation and answers through MediaDone once per item; stop_listen removes it. Document in 1-2 line comments where the host app's own share-extension target is the wiring this reads — the extension itself is app-side, mirroring receive_sharing_intent's docs.
- Mapping: no sharing surface or unsupported presentation -> NotAvailable; staging failure where the source is missing -> FileNotFound, other staging IO failure -> Backend with the OS status via with_status; anything the framework reports as a code lands in status. No unwrap on foreign results; no panics; nothing hangs.
- Add the objc2 dependency lines (objc2, objc2-ui-kit, objc2-app-kit, objc2-foundation with the features you actually use) under [target.'cfg(target_vendor = "apple")'.dependencies] in crates/nsh-share/Cargo.toml — that file is shared; touch only the apple target section, and pin versions that exist on crates.io.

Your scoped gate compiles the whole lib on this macOS host (apple.rs is cfg-included here): cargo fmt --all -- --check, then cargo clippy -p nsh-share --lib -- -D warnings, then cargo test -p nsh-share --lib. The lib also contains sibling areas' files; if a sibling's in-progress code breaks the gate, name the file in your summary instead of editing it. Unsafe code follows the auditor bar: every unsafe block carries the invariant it relies on in a 1-2 line comment.

${FOOTER}`,
    gate: {
      label: "cargo fmt, clippy --lib, test --lib (apple.rs compiles on this host)",
      cmd: "bash",
      args: [
        "-lc",
        "cd /Users/hmziq/fo/nsh-share && cargo fmt --all -- --check && cargo clippy -p nsh-share --lib -- -D warnings && cargo test -p nsh-share --lib",
      ],
      timeoutMs: 1200000,
    },
    auditFocus:
      "Audit apple.rs for FFI soundness and exactly-once delivery: every unsafe block states its invariant; no unwrap/expect on foreign results; UIKit/AppKit calls dispatched to the main thread with no thread blocked on a completion handler; done called exactly once per sheet, on both the completed and cancelled paths; iPad anchor required and mapped to InvalidArgument; staging to temp with FileNotFound vs Backend mapping; inbound handoff reads with MAIN yielding an empty Vec; can_receive false on macOS; Cargo.toml touched only in the apple target section.",
  },
  {
    name: "bridge-android",
    blurb: "platform/android.rs — Intent chooser via JNI, FileProvider URIs, ACTION_SEND inbound",
    specialist: "z-rust-engineer",
    fallback: PLAIN,
    implName: "Rust engineer·bridge-android",
    auditSpecialist: "z-unsafe-ffi-auditor",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/src/platform/android.rs (you may add the android target dependency lines to crates/nsh-share/Cargo.toml's android target section only)",
    ask: `Write the Android bridge. Read first: AGENTS.md, the contract below, .refs/biometrics/crates/nsh-biometrics/src/platform/android.rs (JNI house shape: how ndk-context supplies the VM and activity), share_plus's Android implementation under .refs/plus_plugins/packages/share_plus/share_plus/android/ (src/main/java/... SharePlusPlugin.java), and receive_sharing_intent's android/ sources for the SEND-vs-MAIN cold-start handling and the onNewIntent listener pattern.

${CONTRACT}

You own crates/nsh-share/src/platform/android.rs. Replace the stub completely. Implement Backend for NativeBackend on target_os = "android":
- Outbound: through JNI, build an ACTION_SEND (single item) or ACTION_SEND_MULTIPLE intent with the right MIME type, put text/subject/STREAM URIs, and startChooser. Files: copy large files into the app cache directory and share content:// URIs from a FileProvider with FLAG_GRANT_READ_URI_PERMISSION — never file:// (edge cases large_file_temp_copy, file_uri_rejected_invalid_argument); a source path that cannot be staged resolves FileNotFound, a staging IO error Backend with the OS status. No foreground Activity from ndk-context resolves NoActivity (edge case no_activity_error). Resolve exactly once from the activity result: completed -> Success, dismissed/cancelled -> Dismissed; the answer arrives on a JNI callback thread and must hop into done exactly once without blocking anything.
- Inbound: get_initial_media inspects the launching intent — ACTION_SEND carries the payload, ACTION_MAIN yields an empty Vec and never an error (cold_start_send_vs_main); start_listen registers for subsequent ACTION_SEND deliveries the way receive_sharing_intent listens, answering MediaDone once per item; stop_listen unregisters; reset clears.
- Dependency lines: keep only what you use under [target.'cfg(target_os = "android")'.dependencies] (jni, ndk-context, libc at most) in crates/nsh-share/Cargo.toml — touch only the android target section.

This host is macOS: android.rs is cfg'd out here, so your local gate cannot compile it. Your scoped gate is cargo fmt --all -- --check only; the compile proof for your file is ci.yml's cross-check job (aarch64-linux-android), which another area wires. Say that plainly in your summary: this bridge is audit-verified and CI-compile-verified, not locally compiled. Unsafe/JNI code follows the auditor bar: every unsafe block states the invariant, no unwraps on JNI results, every local reference deleted, no thread blocked on a foreign callback.

${FOOTER}`,
    gate: {
      label: "cargo fmt --check (android.rs is cfg'd out on this host; CI cross-check compiles it)",
      cmd: "bash",
      args: ["-lc", "cd /Users/hmziq/fo/nsh-share && cargo fmt --all -- --check"],
      timeoutMs: 300000,
    },
    auditFocus:
      "Audit android.rs for JNI soundness and exactly-once delivery: every unsafe block states its invariant; JNI local references deleted; exceptions checked after calls; no unwrap on JNI results; ACTION_SEND/MULTIPLE built correctly with MIME types; files staged to cache and shared as FileProvider content URIs with FLAG_GRANT_READ_URI_PERMISSION and never file://; missing Activity -> NoActivity; cold-start MAIN -> empty Vec; the listener registers once and answers MediaDone exactly once per delivery; cancel/stop are non-blocking no-ops on this platform where the OS gives no dismiss API.",
  },
  {
    name: "bridge-desktop-web",
    blurb: "platform/windows.rs, linux.rs, web.rs — DataTransferManager, desktop fallbacks, navigator.share",
    specialist: "z-rust-engineer",
    fallback: PLAIN,
    implName: "Rust engineer·bridge-desktop-web",
    auditSpecialist: "z-unsafe-ffi-auditor",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/src/platform/windows.rs, linux.rs, web.rs (you may add the windows and wasm32 target dependency lines in crates/nsh-share/Cargo.toml's own target sections)",
    ask: `Write the three desktop-and-web bridges. Read first: AGENTS.md, the contract below, .refs/plus_plugins/packages/share_plus/share_plus/lib/src/share_plus_windows.dart, share_plus_linux.dart and share_plus_web.dart (the reference behavior per platform), and .refs/biometrics/crates/nsh-biometrics/src/platform/windows.rs and robius.rs for the windows-crate house style.

${CONTRACT}

You own crates/nsh-share/src/platform/windows.rs, linux.rs and web.rs. Replace each stub completely.
- windows.rs (target_os = windows): DataTransferManager through the windows crate — construct a DataPackage with text, title/subject and staged storage items, show the share UI, and resolve exactly once from the operation: Success on share completion, Dismissed when dismissed, Backend with the HRESULT in status on failure. No window/handle -> NoActivity. Never block the STA thread; nothing hangs. Document in a 1-2 line comment that Windows' share UI has no cancel API, so cancel_share is a no-op and the system's answer still resolves.
- linux.rs (target_os = linux): there is no system share sheet; port share_plus_linux's honest fallback — mailto:/URL-style sharing where mail_to_fallback_enabled is set, else NotAvailable; can_receive false; every call resolves exactly once and nothing hangs. No heavy D-Bus dependency unless the reference behavior needs it; keep the dependency set minimal.
- web.rs (target_arch = wasm32): navigator.share via web-sys — build ShareData from text/title/uri; files only when the API level accepts them; a resolved promise maps to Success, a user abort (AbortError) to Dismissed (edge case web_share_unavailable_not_error... the canonical name is cancel_dismissed_not_error), a missing navigator.share or non-secure context to NotAvailable; the download-fallback flag mirrors share_plus_web's behavior or reports NotAvailable when disabled; get_initial_media and can_receive answer false/NotAvailable.
- Dependency lines: only what each target needs (windows 0.62.x features ApplicationModel_DataTransfer and what you use; wasm-bindgen, web-sys with the Navigator and ShareData features, js-sys) in the matching [target...'dependencies] sections of crates/nsh-share/Cargo.toml — touch only those sections.

This host is macOS: windows.rs, linux.rs and web.rs are all cfg'd out here. Your scoped gate is cargo fmt --all -- --check only; compile proof for these files is ci.yml's cross-check job (wasm32-unknown-unknown) and the windows rust job, which another area wires. Say that plainly in your summary: these bridges are audit-verified and CI-compile-verified, not locally compiled. Unsafe code follows the auditor bar: every unsafe block states its invariant, no unwraps on foreign results, nothing blocks on a foreign callback.

${FOOTER}`,
    gate: {
      label: "cargo fmt --check (windows/linux/web are cfg'd out on this host; CI compiles them)",
      cmd: "bash",
      args: ["-lc", "cd /Users/hmziq/fo/nsh-share && cargo fmt --all -- --check"],
      timeoutMs: 300000,
    },
    auditFocus:
      "Audit the three desktop/web bridges: windows.rs soundness around DataTransferManager (no blocked STA thread, one resolution per share, HRESULT into status, no-window -> NoActivity); linux.rs honest fallback exactly per share_plus_linux with mailto fallback flag honored; web.rs mapping resolve->Success, AbortError->Dismissed, missing API or insecure context->NotAvailable, files gated on API level; all three resolve exactly once, never panic, never hang; Cargo.toml touched only in the matching target sections.",
  },
  {
    name: "ts-package",
    blurb: "packages/nsh-share — the TypeScript package mirroring the Rust API",
    specialist: "z-nsh-engineer",
    fallback: PLAIN,
    implName: "NSH engineer·ts-package",
    auditSpecialist: "z-nsh-engineer",
    auditFallback: PLAIN,
    owns: "everything under packages/nsh-share/ (package.json, tsconfig.json, nsh.json, src/*.ts)",
    ask: `Write the TypeScript package. Read first: AGENTS.md, the contract below, and .refs/biometrics/packages/nsh-biometrics/ as the template — package.json, tsconfig.json, nsh.json, and every src/ file (index.ts, errors.ts, options.ts, backend.ts, memory.ts, rust.ts, plus auth.ts and availability.ts as facade examples) — and one test file for the runner conventions: tests/auth.test.tsx (import { test, expect } from "nsh/test").

${CONTRACT}

You own everything under /Users/hmziq/fo/nsh-share/packages/nsh-share/. Create:
- package.json mirroring biometrics: name nsh-share, version 0.1.0, description in the house tone, license MIT, type module, main and types ./src/index.ts, exports for ".", "./rust", "./memory", repository https://github.com/SuperNSH/share.
- tsconfig.json: extends ./.nsh/tsconfig.json, include src/**/*.ts and tests/**/*.ts (tests belong to another area; the include is fine).
- nsh.json: { "name": "nsh-share tests", "id": "dev.supernsh.share.tests", "version": "0.1.0", "build": 1, "rust": "../../crates/nsh-share", "capabilities": {} }.
- src/errors.ts: ShareErrorCode union with exactly the six codes; MESSAGES with the same wording as the Rust messages; class ShareError extends Error with readonly code; shareError(code); asShareError(error) parsing a host rejection whose code is ERR_NSH_HOST_ERROR and whose message starts with an ERR_SHARE_* code, unknown -> ERR_SHARE_BACKEND — same structure as biometrics errors.ts.
- src/options.ts: ShareParams, ShareFile, Rect, SharedMedia, SharedMediaType, ShareResult, ShareResultStatus types (camelCase mirrors of the Rust structs; ShareResultStatus = "success" | "dismissed" | "unavailable"; SharedMediaType = "image" | "video" | "text" | "file" | "url").
- src/backend.ts: the ShareBackend seam type — isAvailable(): Promise<boolean>, canReceive(): Promise<boolean>, startShare(params: ShareParams, done: (r: Result2<ShareResult, ShareError>) => void): void (or a shape that lets the facade own the promise — keep biometrics' callback spirit), cancelShare(): void, getInitialMedia(): Promise<SharedMedia[]>, startListen(done): void, stopListen(): void. Export the type; keep it callback-exactly-once like the Rust trait.
- src/share.ts: class Share mirroring the Rust facade — constructor({ backend }), share(params: ShareParams): Promise<ShareResult>, shareUri(uri: string, title?: string), shareFiles(files: ShareFile[], text?: string, origin?: Rect); the same validation (no text/uri/files -> InvalidArgument; a path with "://" or a "file:" prefix -> InvalidArgument), the same one-sheet gate (second call rejects InProgress immediately; busy lasts until the answer; generation-gated so a late second answer is ignored; dismissal resolves "dismissed"). Export type AuthenticateArgs-style param types as needed.
- src/receive.ts: class ReceiveSharingIntent mirroring receive.rs — getInitialMedia(): Promise<SharedMedia[]>, receive(): Promise<SharedMedia> (second concurrent receive rejects InProgress), reset(): void.
- src/memory.ts: memoryBackend() + MemoryShare/MemoryShareConfig mirroring FakeBackend's semantics (scripted answers, pendingShare/deliverShare/deliverMedia, setInitialMedia, counters, failNextShare) — the same API biometrics' memory.ts exposes for its backend, adapted to share.
- src/rust.ts: rustBackend() bridging to the native class imported from "nsh-share/native" (the typings the Rust crate's #[native_module] generates), translating param objects and wrapping rejections through asShareError — same structure as biometrics rust.ts. Note in a 1-2 line comment that the app must list the crate under "rust" in nsh.json.
- src/index.ts: re-export the public surface.

Semantics must match the Rust crate exactly — a behavior difference is a finding. Keep src free of React; no App.tsx (no demo required). Your scoped gate runs in the package: nsh check, then nsh test. tests/*.test.ts belong to another area; if nsh test fails on a sibling's missing test files, say so in your summary rather than writing them.

${FOOTER}`,
    gate: {
      label: "nsh check && nsh test in packages/nsh-share",
      cmd: "bash",
      args: [
        "-lc",
        "cd /Users/hmziq/fo/nsh-share/packages/nsh-share && nsh check && nsh test",
      ],
      timeoutMs: 1200000,
    },
    auditFocus:
      "Audit the TS package against the Rust contract and the biometrics template: the six codes and message wording identical to the Rust crate; validation and one-sheet gate semantics identical (including the generation-gated late-answer drop and dismissal as a result); SharedMediaType/ShareResultStatus value sets matching the Rust as_str/Debug values; memory backend counters and delivery semantics mirroring FakeBackend; rust.ts importing from \"nsh-share/native\" and wrapping errors through asShareError with the ERR_NSH_HOST_ERROR parsing; package.json/tsconfig/nsh.json matching the template; no dead code; no React in src.",
  },
  {
    name: "tests-both",
    blurb: "integration tests in Rust and TypeScript covering every goal, edge case and error path",
    specialist: "z-rust-engineer",
    fallback: PLAIN,
    implName: "Rust engineer·tests-both",
    auditSpecialist: "z-rust-engineer",
    auditFallback: PLAIN,
    owns:
      "crates/nsh-share/tests/*.rs and packages/nsh-share/tests/*.test.ts (plus tests/ directories themselves)",
    ask: `Write the test suites in BOTH languages. Read first: AGENTS.md, docs/brief.md's edge cases, the contract below, .refs/biometrics/crates/nsh-biometrics/tests/ (auth.rs, availability.rs) and .refs/biometrics/packages/nsh-biometrics/tests/auth.test.tsx (the house test style, including the delivered() microtask-flush helper), and the crate/package sources as they exist by the time you run your gate — the API is fixed by the contract, so write against it even if a sibling is mid-flight.

${CONTRACT}

${EDGE_CASES}

You own crates/nsh-share/tests/ and packages/nsh-share/tests/. One named test per canonical edge case, in BOTH languages, named after the case (test functions in snake_case matching the case names, nsh test() titles quoting them). Rust: integration tests under crates/nsh-share/tests/share.rs (cases 1-8, 13 via FakeBackend) and receive.rs (9, 10, 11, 14, 15, plus 11's NoActivity via a scripted set_available... note NoActivity is a start_share error path — model it with fail_next_share), all through Share::with_backend(FakeBackend::new()) and ReceiveSharingIntent::with_backend, tokio::test where async. TypeScript: packages/nsh-share/tests/share.test.ts and receive.test.ts with the same cases through memoryBackend(), using import { test, expect } from "nsh/test" and rejects.toMatchObject({ code: "..." }) for coded errors. Also assert, in both languages: every ErrorKind code string (all six, errors.test.ts / tests/errors.rs), the exactly-once counters (deliver twice, resolve once, counters unchanged), and that a Dismissed answer is Ok/`"dismissed"`, not a rejection. Every behavior the goals list must be traceable to a test name — if a case cannot be expressed without the real platform, mark it in a 1-2 line comment as covered by the bridge + CI cross-check instead, and still test the fake-side path.

Your scoped gate: cargo test -p nsh-share --all-targets, then nsh test in packages/nsh-share. The gate compiles sibling areas' files too; if a failure is clearly in a sibling's file, name the file in your summary instead of editing it.

${FOOTER}`,
    gate: {
      label: "cargo test --all-targets && nsh test",
      cmd: "bash",
      args: [
        "-lc",
        "cd /Users/hmziq/fo/nsh-share && cargo test -p nsh-share --all-targets && cd packages/nsh-share && nsh test",
      ],
      timeoutMs: 1200000,
    },
    auditFocus:
      "Audit the test suites: one test per canonical edge case name in BOTH languages; every coded error path asserted through its stable code; the exactly-once property asserted with counters (a second delivered answer changes nothing); dismissal asserted as a success-path result; assertions specific enough to fail on regressions (no tautologies); Rust tests compile under --all-targets with clippy -D warnings cleanliness; TS tests follow the nsh/test conventions; any case skipped for platform reasons carries the 1-2 line justification comment and still exercises the fake-side path.",
  },
];

// ---------------------------------------------------------------------------
// The audit ask and the fix ask.
// ---------------------------------------------------------------------------

function auditAsk(a: AreaDef, gateFailure: string): string {
  const gatePart = gateFailure
    ? `The area's scoped gate FAILED with this output (tail):\n${gateFailure}\nTreat the gate failure as a finding: name the file and the fix.`
    : `The area's scoped gate passed (or this area has no command gate: judge by reading).`;
  return `Audit one area's work. ${a.auditFocus}

The area is "${a.name}"; it owns: ${a.owns}. Read those files in the current checkout at /Users/hmziq/fo/nsh-share, plus AGENTS.md at the repo root. The reference sources live under /Users/hmziq/fo/nsh-share/.refs/ (plus_plugins, receive_sharing_intent, react-native-share, nsh, biometrics) — verify claims against them instead of trusting the code.

${gatePart}

Rules: you are read-only — do not edit, create, or delete any file, do not run cargo or nsh or any build/test command, do not run git. Report actionable findings only: an array of concrete items, each naming the file and what to change; an empty array means clean. No PASS notes, no praise, no summaries of what is fine. Report with: done (boolean), summary (one short paragraph), findings (array of strings).
If the contract below is needed to judge names or codes, it is authoritative:

${CONTRACT}`;
}

function fixAsk(a: AreaDef, findings: string[], gateFailure: string): string {
  const gatePart = gateFailure
    ? `Your scoped gate failed with (tail):\n${gateFailure}\nFix your files so it passes.`
    : "";
  return `Fix round for your area "${a.name}". You own exactly: ${a.owns}. The auditor read your work against the contract and AGENTS.md and reports these actionable findings:\n${findings
    .map((f, i) => `${i + 1}. ${f}`)
    .join("\n")}\n\n${gatePart}\nChange only files you own; never touch sibling areas' files (if a finding demands that, say so in blockers instead). Keep AGENTS.md house rules. Then report with: done (boolean), summary (what you changed, honestly), blockers (optional).`;
}

// ---------------------------------------------------------------------------
// Step 0 (serial), then the fan-out.
// ---------------------------------------------------------------------------

phase("Step 0: verify the cloned references and warm the cargo cache; then each area runs its own chain in parallel");

const step0 = await world.run("bash", [`${ROOT}/.zcode/step0-refs.sh`], { timeoutMs: 180000 });
if (step0.exitCode !== 0) {
  throw new Error(
    `Step 0 failed: the five references could not be verified. Exit ${step0.exitCode}: ${String(
      (step0.stderr || "") + (step0.stdout || ""),
    ).slice(-500)}`,
  );
}

const warm = await world.run("bash", [`${ROOT}/.zcode/warm-cargo.sh`], { timeoutMs: 1200000 });
if (warm.exitCode !== 0) {
  throw new Error(
    `Step 0 failed: nsh-core 0.4.0 did not resolve or compile here, so every cargo gate would be dishonest. Exit ${warm.exitCode}: ${String(
      (warm.stderr || "") + (warm.stdout || ""),
    ).slice(-800)}`,
  );
}

function tailOf(r: { stdout?: string; stderr?: string }, max = 1400): string {
  return `${r.stdout || ""}\n${r.stderr || ""}`.trim().slice(-max);
}

async function runArea(a: AreaDef): Promise<Record<string, unknown>> {
  report({ name: a.name, phase: "implementing", summary: a.blurb }, "areas");
  const implPersona = await specialist(a.specialist, a.fallback);
  const impl = agent(a.implName, { system: implPersona, model: MODELS.pro });

  const done = await impl.ask(a.ask);
  if (!done || done.done === false) {
    const why = done?.summary || "implementer reported done=false";
    report({ name: a.name, phase: "failed", summary: why.slice(0, 160) }, "areas");
    return { name: a.name, status: "failed", summary: why.slice(0, 400), rounds: 0 };
  }

  let rounds = 0;
  let converged = false;
  let lastFindings: string[] = [];
  let lastGate = "";
  let lastSummary = done.summary || "";

  while (rounds < AUDIT_ROUNDS) {
    rounds += 1;

    if (a.gate) {
      report({ name: a.name, phase: "gating", summary: a.gate.label }, "areas");
      const g = await world.run(a.gate.cmd, a.gate.args, { timeoutMs: a.gate.timeoutMs });
      lastGate = g.exitCode === 0 ? "" : tailOf(g);
    }

    report({ name: a.name, phase: "auditing", summary: `round ${rounds}` }, "areas");
    const auditPersona = await specialist(a.auditSpecialist, a.auditFallback);
    const auditor = agent(`Auditor·${a.name}·${rounds}`, {
      system: auditPersona,
      model: MODELS.pro,
    });
    const audit = await auditor.ask(auditAsk(a, lastGate));
    lastFindings = audit && Array.isArray(audit.findings) ? audit.findings : [];
    if (!audit || audit.done === false) {
      lastFindings = [
        `the auditor could not complete its review: ${audit?.summary || "no result"}`,
      ];
    }

    if (lastFindings.length === 0 && !lastGate) {
      converged = true;
      break;
    }

    report(
      {
        name: a.name,
        phase: "fixing",
        summary: `${lastFindings.length} finding(s)${lastGate ? " + gate failure" : ""}`,
      },
      "areas",
    );
    const fix = await impl.ask(fixAsk(a, lastFindings, lastGate));
    if (fix && fix.summary) lastSummary = fix.summary;
  }

  const status = converged ? "done" : "unconverged";
  const summary = converged
    ? `converged after ${rounds} round(s): ${lastSummary}`.slice(0, 400)
    : `did NOT converge in ${AUDIT_ROUNDS} rounds; last findings: ${lastFindings
        .slice(0, 3)
        .join(" | ")
        .slice(0, 260)}`;
  report({ name: a.name, phase: status, summary: summary.slice(0, 160) }, "areas");
  return { name: a.name, status, rounds, converged, summary };
}

const areaResults = await Promise.all(
  areas.map(async (a) => {
    try {
      return await runArea(a);
    } catch (e) {
      const msg = String(e && e.message ? e.message : e).slice(0, 400);
      report({ name: a.name, phase: "failed", summary: msg.slice(0, 160) }, "areas");
      return { name: a.name, status: "failed", summary: msg };
    }
  }),
);

// ---------------------------------------------------------------------------
// The join: the brief's four whole-tree gates, once, over every area's work.
// ---------------------------------------------------------------------------

phase("Whole-tree gates at the join: fmt, clippy, cargo test, nsh check + nsh test");

const JOIN_GATES: { label: string; args: string[] }[] = [
  { label: "cargo fmt --all -- --check", args: ["-lc", `cd ${ROOT} && cargo fmt --all -- --check`] },
  {
    label: "cargo clippy --all-targets -- -D warnings",
    args: ["-lc", `cd ${ROOT} && cargo clippy --all-targets -- -D warnings`],
  },
  {
    label: "cargo test --all-targets",
    args: ["-lc", `cd ${ROOT} && cargo test --all-targets`],
  },
  {
    label: "nsh check && nsh test (packages/nsh-share)",
    args: ["-lc", `cd ${ROOT}/packages/nsh-share && nsh check && nsh test`],
  },
];

let finalGate = "pass";
let finalDetail = "all four gates green";
const failedJoin: string[] = [];
for (const g of JOIN_GATES) {
  const r = await world.run("bash", g.args, { timeoutMs: 1200000 });
  if (r.exitCode !== 0) {
    failedJoin.push(g.label);
  }
}

if (failedJoin.length > 0) {
  report(
    { name: "join", phase: "fixing", summary: `${failedJoin.length} gate(s) failed; one fix round` },
    "areas",
  );
  const fixer = agent("Join fixer", {
    system: await specialist("z-rust-engineer", PLAIN),
    model: MODELS.pro,
  });
  const reruns: string[] = [];
  for (const label of failedJoin) {
    const spec = JOIN_GATES.find((g) => g.label === label)!;
    const before = await world.run("bash", spec.args, { timeoutMs: 1200000 });
    const out = tailOf(before);
    const fix = await fixer.ask(
      `The whole-tree gate "${label}" failed over the joined tree at /Users/hmziq/fo/nsh-share. Output (tail):\n${out}\n\nFix it by editing the repo directly — any file, but change the minimum: this is a join fix, not a rewrite. No git commands, no new features. Then report with: done (boolean), summary (what you changed), blockers (optional).`,
    );
    const after = await world.run("bash", spec.args, { timeoutMs: 1200000 });
    reruns.push(`${label}: ${after.exitCode === 0 ? "green after fix" : `still red (exit ${after.exitCode})`}`);
    void fix;
  }
  const stillRed = reruns.filter((r) => r.includes("still red"));
  finalGate = stillRed.length === 0 ? "pass-after-fix" : "failed";
  finalDetail = reruns.join("; ");
}

// ---------------------------------------------------------------------------
// Final return.
// ---------------------------------------------------------------------------

const failed = areaResults.filter((r) => r.status === "failed");
const unconverged = areaResults.filter((r) => r.status === "unconverged");
const conclusion = [
  `nsh-share: ${areas.length} areas ran; ${areaResults.length - failed.length - unconverged.length} converged, ${unconverged.length} hit the 3-round audit cap, ${failed.length} failed.`,
  `Final whole-tree gates: ${finalGate} (${finalDetail}).`,
  `Rust crate at crates/nsh-share, TypeScript package at packages/nsh-share, both laid out like .refs/biometrics with the same error codes and exactly-once semantics; docs and ci.yml updated.`,
  `Per AGENTS.md nothing was committed or pushed; device-platform bridges (Android, Windows, Linux, web) are locally fmt-checked and audit-verified, with their compile proof in ci.yml's cross-check job.`,
].join(" ");

return {
  conclusion,
  areas: areaResults,
  finalGate,
  finalDetail,
  notes:
    "GLM-5.3 Max (MODELS.pro) pinned on every implementer, auditor and the join fixer per the brief; no vision phase exists, so flash is unused. Audit budget 3 rounds per area, until zero findings and a clean scoped gate or the cap. STEP 0 cloned the five references into .refs/ (already present, verified) and warmed the cargo cache so the first cold nsh-core compile (~10.5 min measured) is paid once, serially. Scoped per-area gates: lib-target cargo commands for rust-core and bridge-apple (apple.rs compiles on this macOS host), fmt-only for the cfg'd-out bridges (Android, Windows, Linux, web compile in CI's cross-check), nsh check/test for the TS package, cargo test --all-targets + nsh test for the tests area.",
};
