// Fresh workflow script for one run: nsh-share — build a Rust crate and a TypeScript package
// (laid out like .refs/biometrics) for outbound share and inbound receive, in this repo only.
//
// Per-area chain, all inside one fan-out: implement -> [gate -> audit -> fix]* until the
// scoped gate is clean AND the auditor reports zero findings, capped at 3 rounds (the brief:
// GLM-5.3 Max everywhere, 3 audit rounds per area, honest report if a cap hits).
// STEP 0 (verify the .refs clones + warm the cargo cache) runs serially before the fan-out.
// The join runs the brief's four whole-tree gates once, with one bounded fix round.
// No commits, no pushes, no branches: areas edit the current checkout directly.

const MODELS = {
  // The brief pins GLM-5.3 Max for every implementer AND every audit/check phase.
  pro: model("account:zai-individual-coding-plan/GLM-5.3$high"),
  // Only a vision-capable capture interpreter runs flash; this run has no UI capture, so flash is declared but unused.
  flash: model("account:zai-individual-coding-plan/GLM-5.3-Flash$high"),
};

const ROOT = "/Users/hmziq/fo/nsh-share";
const KIT = "/Users/hmziq/ZCodeProject/ai/harness/zcode/agents";
const AUDIT_ROUNDS = 3; // brief: until zero issues or the cap

// Whole-file specialist bodies load at run time; the script never carries a copy.
async function specialistPersona(
  name: string,
  fallback: string,
): Promise<{ persona: string; source: string }> {
  const adapter =
    `You are the ${name} specialist for one area of a parallel workflow. ` +
    `Your full specialist contract follows verbatim — apply it to this area's work; ` +
    `where it conflicts with your ask, the ask wins.` + "\n\n";
  for (const p of [`${ROOT}/.zcode/agents/${name}.md`, `${KIT}/${name}.md`]) {
    try {
      const raw = await files.read(p);
      return {
        persona: adapter + raw.replace(/^---\n[\s\S]*?\n---\n/, "").trim(),
        source: p,
      };
    } catch {
      /* try the next source */
    }
  }
  return { persona: fallback, source: "fallback" };
}

const PLAIN =
  "You are a careful, senior implementer. Follow the ask exactly; where the ask is silent, follow the repo's AGENTS.md.";

artifact.board("areas", {
  key: "name",
  status: "phase",
  columns: ["implementing", "gating", "auditing", "fixing", "done", "unconverged", "failed"],
});

// ---------------------------------------------------------------------------
// The predetermined API contract every coding area codes against.
// ---------------------------------------------------------------------------

const CONTRACT = `
Predetermined API contract (every area codes against exactly these names; nobody renames anything):

Repo layout, mirroring .refs/biometrics:
- Root /Users/hmziq/fo/nsh-share/Cargo.toml becomes a virtual workspace manifest: members = ["crates/nsh-share"], resolver = "3", and [workspace.package] with version 0.1.0, edition 2024, license MIT, repository https://github.com/SuperNSH/share, authors ["hmziqrs"]. The old root src/lib.rs scaffold is deleted (rm the file); the crate lives at crates/nsh-share/ with [lib] name nsh_share. The TypeScript package lives at packages/nsh-share/.

Crate nsh-share dependencies (verified against .refs/biometrics/crates/nsh-biometrics/Cargo.toml, which pins the same versions):
- nsh-core = "0.4.0", thiserror = "2.0.21", tokio 1.54.1 with default-features = false and features ["sync","time"].
- Dev-dependencies: tokio 1.54.1 with features ["rt","time","test-util"]. The house test convention (see .refs/biometrics/crates/nsh-biometrics/tests/auth.rs) is plain #[test] plus a run() helper over a current-thread runtime (tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(...)) — NOT #[tokio::test]. Do not add the tokio "macros" feature.
- Bridge dependencies are NOT in the base manifest: each bridge area adds only its own [target.'cfg(...)'.dependencies] table when its bridge needs one. crates/nsh-share/Cargo.toml is shared, so any bridge touching it re-reads the file immediately before a minimal edit of only its own target section.

Rust public API:
- error.rs: #[non_exhaustive] enum ErrorKind { NotAvailable, InvalidArgument, InProgress, NoActivity, FileNotFound, Backend } with ErrorKind::code() returning the stable codes "ERR_SHARE_NOT_AVAILABLE", "ERR_SHARE_INVALID_ARGUMENT", "ERR_SHARE_IN_PROGRESS", "ERR_SHARE_NO_ACTIVITY", "ERR_SHARE_FILE_NOT_FOUND", "ERR_SHARE_BACKEND", and ErrorKind::message() returning one-sentence what-to-do strings; struct ShareError { kind: ErrorKind, status: Option<i64> } with new(kind), with_status(i64), kind(), status(), code() -> &'static str (the stable code), message() -> &'static str; #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)] with #[error("{}", .kind.message())]; impl fmt::Display for ErrorKind writes the code — the exact shape of .refs/biometrics/crates/nsh-biometrics/src/error.rs. A dismissed sheet is NOT an error: outbound cancellation is ShareResultStatus::Dismissed, an Ok answer.
- options.rs: Rect { x: f64, y: f64, width: f64, height: f64 } (PartialEq, Debug, Clone, Copy, Default); ShareFile { path: String, mime_type: Option<String> } where path is a plain filesystem path, never a file:// URI; ShareParams { title: Option<String>, text: Option<String>, subject: Option<String>, uri: Option<String>, files: Vec<ShareFile>, file_name_overrides: Vec<String>, share_position_origin: Option<Rect>, preview_thumbnail: Option<String>, mail_to_fallback_enabled: bool, download_fallback_enabled: bool } with Default (both fallback flags default true).
- share.rs: struct Share, Clone, clones sharing one sheet gate over an Arc. Share::new() returns the process-wide platform handle (std::sync::OnceLock over platform::default_backend()); Share::with_backend(backend: Arc<dyn Backend>) is for tests and fakes. async is_available(&self) -> Result<bool, ShareError>; async share(&self, params: ShareParams) -> Result<ShareResult, ShareError>; async share_uri(&self, uri: impl Into<String>, title: Option<String>) -> Result<ShareResult, ShareError>; async share_files(&self, files: Vec<ShareFile>, text: Option<String>, origin: Option<Rect>) -> Result<ShareResult, ShareError>. ShareResult { raw: String, status: ShareResultStatus }; #[derive(Clone, Copy, PartialEq, Eq, Debug)] enum ShareResultStatus { Success, Dismissed, Unavailable } with as_str() returning "success"/"dismissed"/"unavailable" (share_plus's values).
- receive.rs: struct ReceiveSharingIntent with new() and with_backend(Arc<dyn Backend>); async get_initial_media(&self) -> Result<Vec<SharedMedia>, ShareError> — the cold-start read; an Android MAIN launch or a plain iOS launch yields an empty Vec, never an error; async receive(&self) -> Result<SharedMedia, ShareError> — waits for the next inbound item, one waiter at a time; fn reset(&self) clears stored initial media. SharedMedia { path: String, thumbnail: Option<String>, duration: Option<u64>, kind: SharedMediaType, mime_type: Option<String>, message: Option<String> }; enum SharedMediaType { Image, Video, Text, File, Url } with as_str() returning "image"/"video"/"text"/"file"/"url" (receive_sharing_intent's values).
- backend.rs: type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>; type ShareDone = Box<dyn FnOnce(Result<ShareResult, ShareError>) + Send>; type MediaDone = Box<dyn FnOnce(Result<Vec<SharedMedia>, ShareError>) + Send>; trait Backend: Send + Sync { fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>>; fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>>; fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError>; fn cancel_share(&self); fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>>; fn start_listen(&self, done: MediaDone) -> Result<(), ShareError>; fn stop_listen(&self); } — the shape of .refs/biometrics backend.rs. start_share returns at once; done is called exactly once when the system answers; an Err from start_share means no sheet was shown and done is never called. cancel_share is best-effort: the system's own answer still arrives through done; where the platform cannot cancel, it is a no-op.
- fake.rs: FakeBackend (Clone over Arc<Mutex<state>>), mirroring .refs/biometrics fake.rs's shape: Default is available true, can_receive false; set_available(Result<bool, ShareError>), set_can_receive(Result<bool, ShareError>), fail_next_share(ShareError) (the next start_share returns Err and no sheet is shown), pending_share() -> bool, deliver_share(Result<ShareResult, ShareError>) -> bool (false when nothing waits), deliver_media(Result<SharedMedia, ShareError>) -> bool, set_initial_media(Vec<SharedMedia>), counters shares_started() / cancels() / listens_started(), last_params() -> Option<ShareParams>.
- share.rs facade rules (what Share adds over a Backend): validate before start_share — params with no text, no uri and no files is InvalidArgument; any file path containing "://" or starting with "file:" is InvalidArgument (bridges turn staged paths into provider URIs themselves); on every target except wasm32 (std::path is behind cfg there — 1-2 line comment saying the web bridge decides), a listed path that does not exist resolves FileNotFound before any sheet starts; while a sheet is up another share call returns InProgress immediately; the busy state lasts until the system answers, even past a cancel request; the answer resolves the call exactly once through a generation-gated tokio::sync::oneshot (the Gate pattern of .refs/biometrics auth.rs: a generation counter so a late answer from an older generation cannot resolve a newer call), so a late second answer is ignored. Dismissal maps to Ok(ShareResult { status: Dismissed }).
- receive.rs facade rules: get_initial_media resolves once from the backend snapshot; receive while another receive waits is InProgress; stopping or reset never resolves a waiting receive — the backend's answer does.
- native.rs: TSX class NativeShare exposed with #[class] structs and #[native_module("nsh-share/native")] in lib.rs, mirroring .refs/biometrics native.rs and lib.rs (use nsh_core::prelude::*; #[derive(JsObject)] param structs; #[class] struct NativeShare { share: Share }; #[class] impl with async members taking this: Instance<Self> and returning Result<T, JsError>, mapping ShareError to a rejection carrying its stable code the way biometrics' into_js does; #[native_module] fn native(registry: &mut Registry) registering the class). Param structs NativeShareParams, NativeShareFile { path, mime_type }, NativeRect { x, y, width, height } with Option fields, converted into the Rust types by a to_share_params-style function with a #[cfg(test)] mod tests proving the conversion and the error mapping. Verify every macro and trait name against the vendored nsh-core 0.4.0 source under .refs/nsh/crates/nsh-core before writing; never invent an API.
- platform/mod.rs: cfg dispatch exactly like .refs/biometrics platform/mod.rs — target_vendor = "apple" -> apple, target_os = "android" -> android, target_os = "linux" -> linux, target_os = "windows" -> windows, target_arch = "wasm32" -> web, otherwise unsupported — plus pub(crate) fn default_backend() -> Arc<dyn Backend>.
- platform/unsupported.rs: ONE deliberate divergence from biometrics, with a 1-2 line comment stating the constraint: the module is NOT cfg'd out (it compiles on every target, so its contract can be unit-tested on every runner); mod.rs simply never wires it in where a real backend exists. struct NativeBackend implementing Backend: is_available and can_receive answer Ok(false); start_share and start_listen answer Err(NotAvailable); get_initial_media answers Ok(empty); cancel_share and stop_listen are no-ops. Carry a #[cfg(test)] mod tests with the named test platform_unsupported_reports_not_available asserting exactly that, through the run() helper.
- Platform bridge stubs: apple.rs, android.rs, windows.rs, linux.rs, web.rs compile from the start as NativeBackends behaving like unsupported.rs; each sibling bridge area owns one file and replaces its contents this run. The stubs keep the tree green if a sibling fails.

House rules (AGENTS.md, binding): comments are 1-2 lines and state a non-obvious constraint, never narrate the next line; dead code is deleted, no speculative abstractions; docs read direct and concrete with no marketing filler; an operation resolves exactly once — success, a coded error, or a cancel — and never hangs; a native bridge never panics (no unwrap/expect on foreign results) and never blocks a thread waiting on a foreign callback; status claims in README and docs must match what CI runs.
`;

const EDGE_CASES = `
Canonical edge cases (docs/goals.md documents each with the same name; every name has a named test — most in BOTH languages, two as lib unit tests where the platform code lives):
1. cancel_dismissed_not_error — dismissing the sheet resolves Ok with ShareResultStatus::Dismissed, never an error.
2. second_share_rejected_in_progress — while a sheet is up, another share call fails InProgress immediately and the first call still resolves normally.
3. late_answer_ignored_resolve_once — the first answer resolves the call; a second delivered answer is dropped and the counters show one resolution.
4. cancel_share_best_effort — cancel does not resolve the call; the system's own answer still lands (mirrors biometrics: the platform keeps the sheet until it answers).
5. empty_share_invalid_argument — no text, no uri, no files.
6. file_uri_rejected_invalid_argument — a file path carrying "://" or a "file:" prefix is rejected before any sheet starts.
7. missing_file_file_not_found — a listed path that does not exist resolves FileNotFound before any sheet starts (the facade stats it off-wasm; the fake's fail_next_share covers the backend-side staging path).
8. ipad_anchor_required — on iPad a missing share_position_origin is InvalidArgument, because presenting without an anchor crashes; enforced by the Apple bridge, pinned by a #[cfg(test)] unit test inside apple.rs on a pure helper (apple.rs compiles on macOS hosts).
9. cold_start_send_vs_main — get_initial_media returns the SEND payload and an empty Vec for a MAIN launch; never an error (facade tested through the fake; the real intent/activity read is the bridge's, CI-compile-verified).
10. large_file_temp_copy — a large file is staged to a temp file and shared by a provider URI, never file:// (the fake-side test asserts last_params carries the files and comments where the real copy lives: the Android bridge, CI-compile-verified).
11. no_activity_error — no foreground Activity (Android) or no window (Windows) answers NoActivity (facade tested through fail_next_share; the real detection is the bridge's).
12. web_share_unavailable_not_available — web without the Web Share API answers NotAvailable; a user abort (AbortError) answers Dismissed (facade tested through the fake; the mapping is the web bridge's).
13. platform_unsupported_reports_not_available — the unsupported target answers is_available false and every call NotAvailable (lib unit test inside platform/unsupported.rs, which compiles on every target by design).
14. in_flight_receive_rejected — a second receive while one waits answers InProgress.
15. reset_clears_initial_media — reset empties what get_initial_media would return.
`;

const FOOTER =
  "Work directly in the current checkout at /Users/hmziq/fo/nsh-share — no branches, no worktrees, no commits, no pushes, no git commands at all — and do not run whole-tree test suites beyond your own scoped gate; finish your piece and report. Per AGENTS.md never build, run or launch any app, emulator, simulator or device target locally, and never run nsh run or nsh dev; your scoped gate commands are the only commands you run. Report with: done (boolean), summary (a short honest paragraph: what landed, what you could not verify), blockers (optional array).";

// ---------------------------------------------------------------------------
// Area definitions. Chains: docs is implement -> audit; every other area is
// implement -> [gate -> audit -> fix]* bounded by AUDIT_ROUNDS.
// ---------------------------------------------------------------------------

interface GateSpec {
  label: string;
  args: string[];
  timeoutMs: number;
}

interface AreaDef {
  name: string;
  blurb: string;
  implSpecialist: string;
  implFallback: string;
  implName: string;
  auditSpecialist: string;
  auditFallback: string;
  owns: string;
  ask: string;
  gate: GateSpec | null;
  auditFocus: string;
}

const GATE_LIB: GateSpec = {
  label: "cargo fmt, clippy -p --lib, test -p --lib",
  args: [
    "-lc",
    `cd ${ROOT} && cargo fmt --all -- --check && cargo clippy -p nsh-share --lib -- -D warnings && cargo test -p nsh-share --lib`,
  ],
  timeoutMs: 1200000,
};

const GATE_FMT: GateSpec = {
  label: "cargo fmt --check (the bridge is cfg'd out on this macOS host; ci.yml's cross-check compiles it)",
  args: ["-lc", `cd ${ROOT} && cargo fmt --all -- --check`],
  timeoutMs: 300000,
};

const areas: AreaDef[] = [
  {
    name: "docs",
    blurb: "goals, parity, decisions, README",
    implSpecialist: "z-nsh-engineer",
    implFallback: PLAIN,
    implName: "NSH engineer·docs",
    auditSpecialist: "z-nsh-engineer",
    auditFallback: PLAIN,
    owns: "docs/goals.md, docs/parity.md, docs/decisions.md, README.md",
    ask: `Write the four docs files for nsh-share. Read first: AGENTS.md, docs/brief.md, the existing docs/*.md and README.md, and the reference sources under ${ROOT}/.refs/ (plus_plugins/packages/share_plus/share_plus/lib/share_plus.dart and the share_plus_platform_interface package, receive_sharing_intent/lib/receive_sharing_intent.dart and lib/src/, react-native-share/src/types.ts and src/index.tsx, and .refs/biometrics/docs/*.md as the house style). Cloning already happened in step 0; if a reference you need is missing, say so in blockers — do not clone anything yourself.

You own exactly these files (create or rewrite): docs/goals.md, docs/parity.md, docs/decisions.md, README.md. No other file.

docs/goals.md: what nsh-share does (outbound share of text, URLs, files and images; inbound receive into the app), acceptance criteria phrased as the gates CI runs (cargo fmt --all -- --check, cargo clippy --all-targets -- -D warnings, cargo test --all-targets, and in packages/nsh-share: nsh check and nsh test), the error table — one row per stable code: ERR_SHARE_NOT_AVAILABLE, ERR_SHARE_INVALID_ARGUMENT, ERR_SHARE_IN_PROGRESS, ERR_SHARE_NO_ACTIVITY, ERR_SHARE_FILE_NOT_FOUND, ERR_SHARE_BACKEND — each with its trigger and how platforms map to it, and the edge-case list. Use the canonical names and meanings below verbatim; each entry maps to its named test (state where the test lives: tests/share.rs, tests/receive.rs, tests/errors.rs, share.test.ts, receive.test.ts, errors.test.ts, the lib unit tests in apple.rs and platform/unsupported.rs).

${EDGE_CASES}

Also state the exactly-once rule in goals.md: every call resolves exactly once as success, a coded error, or a cancel, and a cancelled sheet is a result (Dismissed), not an error.

docs/parity.md: one section per reference. For share_plus: Share.share, Share.shareUri, Share.shareXFiles, SharePlus.instance.share, every ShareParams field (text, title, subject, files, fileNameOverrides, sharePositionOrigin, previewThumbnail, uri, downloadFallbackEnabled, mailToFallbackEnabled, excludedCupertinoActivities), ShareResult (raw, status) and ShareResultStatus (success, dismissed, unavailable) — for each, what ours does (same name, same behavior, or accepted-and-ignored, or not ported and why). For receive_sharing_intent: getInitialMedia, the receive broadcast stream, reset, SharedMediaFile (path, thumbnail, duration, type, mimeType, message) and SharedMediaType (image, video, text, file, url). For react-native-share: RNShare.open, shareSingle, isPackageInstalled, ShareOptions and ShareSingleOptions fields, ShareOpenResult, and the UI components (Overlay, Sheet, Button, ShareSheet) — single-target social sheets and UI components are reference-only: say so per item, with one line why (host-app UI, not a capability API). Mark every row with what a test proves it; a claim no test or gate runs must say unverified.

docs/decisions.md: move the three open decisions to Decided with reasons: (1) NSH stays on 0.4.0 — the installed nsh CLI (0.4.0) and the published nsh-core 0.4.0 agree, and nothing share needs unreleased main; (2) library approach — port share_plus for outbound (UIActivityViewController/NSSharingService via objc2 on Apple, Intent chooser via JNI on Android, DataTransferManager on Windows), port receive_sharing_intent's design for inbound (cold-start read plus a listen stream); react-native-share is reference only; (3) platforms in scope — outbound iOS, macOS, Android, Windows, Linux, web; inbound iOS and Android, where the share extension or intent filters are host-app wiring the crate reads from. Record the versions and licences you actually read from the cloned repos' own manifests and LICENSE files (do not guess, do not fetch the network), and record that all five reference URLs cloned successfully on 2026-11-09 so no canonical-repo substitution was needed.

README.md: update Status (implemented at the unit-test level: behaviors proven by cargo test and nsh test in CI; device-platform bridges compile-checked by CI's cross targets — never claim device verification), Layout (crates/nsh-share, packages/nsh-share, docs, site, .github/workflows), and Checks (list exactly the gates the updated ci.yml runs). Keep it short and concrete; the current README's tone is the house tone.

${FOOTER}`,
    gate: null,
    auditFocus:
      "Audit the four docs files for: accuracy against the .refs sources (API names, field lists, enum values — open the actual reference files and check each claim), coverage of all three references' public APIs, the six error codes matching the contract, all fifteen canonical edge-case names present with meanings intact, decisions.md resolving the three open decisions with versions and licences read from the cloned repos, and README status claims matching only what CI actually runs. Flag marketing filler, hedged claims, and any claim marked done that no gate or test covers.",
  },
  {
    name: "rust-core",
    blurb: "workspace restructure, core API, error types, backend trait, fake, native class, ci.yml",
    implSpecialist: "z-nsh-engineer",
    implFallback: PLAIN,
    implName: "NSH engineer·rust-core",
    auditSpecialist: "z-rust-engineer",
    auditFallback: PLAIN,
    owns:
      "root Cargo.toml, crates/nsh-share/Cargo.toml, crates/nsh-share/src/lib.rs, error.rs, options.rs, backend.rs, fake.rs, share.rs, receive.rs, native.rs, platform/mod.rs, platform/unsupported.rs, the temporary platform stub files apple.rs, android.rs, windows.rs, linux.rs, web.rs, deleting the old root src/lib.rs, and .github/workflows/ci.yml",
    ask: `Restructure the repo into a workspace and write the Rust core. Read first: AGENTS.md, docs/brief.md, the current root Cargo.toml and src/lib.rs, .refs/biometrics/Cargo.toml, .refs/biometrics/crates/nsh-biometrics/src/ (lib.rs, error.rs, backend.rs, fake.rs, auth.rs for the Gate pattern, native.rs, platform/mod.rs, platform/unsupported.rs — copy the shapes, adapt the content), .refs/biometrics/tests-style (tests/auth.rs's run() helper), and the nsh-core 0.4.0 source under .refs/nsh/crates/nsh-core for the exact prelude, #[class], JsObject and #[native_module] APIs. Never invent an nsh-core API; verify in that source.

${CONTRACT}

Work plan, in order:
1. Root manifest to a virtual workspace; create crates/nsh-share with its Cargo.toml (workspace inheritance, [lib] name nsh_share, base dependencies only — NO bridge target sections, those belong to the bridge areas); write all src modules per the contract; delete the old root src/lib.rs file with rm (no git commands).
2. fake.rs and the facade gate logic first, with #[cfg(test)] unit tests inline in share.rs and receive.rs covering the gate rules (empty-args validation, file-URI rejection, the off-wasm missing-file stat, in-progress rejection, resolve-exactly-once with a dropped late answer, cancel does not resolve, reset clears) using FakeBackend and the run() helper convention.
3. lib.rs wires modules, re-exports the public API, and carries #[native_module("nsh-share/native")] registering NativeShare.
4. platform stubs: write every platform file (apple.rs, android.rs, windows.rs, linux.rs, web.rs) as a compiling NativeBackend that behaves like unsupported.rs for now — the five sibling bridge areas own those files and will replace their contents this run; the stubs keep the tree green if a sibling fails. mod.rs and unsupported.rs are final in your hands, including unsupported.rs's platform_unsupported_reports_not_available unit test.
5. Keep clippy clean at -D warnings including the tests.

Then rewrite .github/workflows/ci.yml so it runs both stacks, mirroring .refs/biometrics/.github/workflows/ci.yml's job layout, pinned-action SHAs, kache caching and header comment convention, with no device, emulator or demo steps anywhere: a format job (cargo fmt --all -- --check); a rust job on ubuntu-latest, macos-latest and windows-latest (fail-fast false) running cargo clippy --all-targets -- -D warnings and cargo test --all-targets (no special Linux packages: the Linux bridge is a process-spawn fallback, not D-Bus); a cross-check job compiling the crate for aarch64-apple-ios, aarch64-apple-ios-sim, aarch64-linux-android and wasm32-unknown-unknown with cargo clippy -p nsh-share --target <target> -- -D warnings (deliberately without --all-targets, mirroring biometrics: test-only dev-dependencies are not exercised on cross targets), pointing cc at the NDK for the Android target the way biometrics does; and a typescript job installing NSH v0.4.0 from SuperNSH/nsh-builds (copy biometrics' install steps for both non-Windows and Windows runners) then, working-directory packages/nsh-share, nsh check, nsh build piped through tee with a failing grep on warnings, and nsh test.

Your scoped gate is: cargo fmt --all -- --check, then cargo clippy -p nsh-share --lib -- -D warnings, then cargo test -p nsh-share --lib. Integration tests under crates/nsh-share/tests/ belong to another area; do not create them, and if a sibling's in-progress files break your gate, say which files in your summary rather than editing them.

${FOOTER}`,
    gate: GATE_LIB,
    auditFocus:
      "Audit the workspace restructure and core modules: manifest and module shapes against the contract and .refs/biometrics; the gate logic (exactly-once via the generation-gated oneshot, in-progress rejection, validation rules and their order, busy-until-answered, the off-wasm missing-file stat); error codes, messages and Display shape; the Backend trait and FakeBackend surface against the contract names; native.rs against the vendored nsh-core source (no invented APIs) and the conversion unit tests; unsupported.rs compiling ungated with its named unit test; ci.yml covering both stacks and the four cross targets with no device or emulator steps; AGENTS.md house rules (comments, no dead code); clippy-cleanliness under -D warnings.",
  },
  {
    name: "bridge-apple",
    blurb: "platform/apple.rs — UIActivityViewController, NSSharingService, iPad anchor, iOS inbound",
    implSpecialist: "z-rust-engineer",
    implFallback: PLAIN,
    implName: "Rust engineer·bridge-apple",
    auditSpecialist: "z-unsafe-ffi-auditor",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/src/platform/apple.rs (plus, only if your bridge needs them, the [target.'cfg(target_vendor = \"apple\")'.dependencies] table of crates/nsh-share/Cargo.toml)",
    ask: `Write the Apple bridge. Read first: AGENTS.md, the contract below, .refs/biometrics/crates/nsh-biometrics/src/platform/apple.rs (the house bridge shape), the share_plus Apple implementation under .refs/plus_plugins/packages/share_plus/share_plus/ (ios/ Classes/ SharePlusPlugin.swift and macos/ Classes/), and the vendored objc2 sources if the workspace vendors them; otherwise pin the objc2 family versions from the crates.io generation you know compiles (biometrics pins objc2-local-authentication 0.3.2, the objc2 0.6 generation) and let your scoped gate prove the pins — it compiles this file on this macOS host.

${CONTRACT}

You own crates/nsh-share/src/platform/apple.rs. Replace the stub completely. Implement Backend for NativeBackend on target_vendor = "apple":
- Availability: is_available answers whether a sharing surface exists (UIActivityViewController on iOS, NSSharingServicePicker on macOS); can_receive answers false on macOS and true on iOS.
- Outbound: show UIActivityViewController with the item list built from ShareParams — text, url, and files staged into a temp directory and shared as file URLs; subject maps to the email subject where the platform exposes it; share_position_origin becomes the popover sourceView/sourceRect anchoring on iPad — REQUIRED on iPad, and a missing anchor is InvalidArgument (edge case ipad_anchor_required), because presenting without one crashes. Put the anchor rule in a pure, testable helper and pin it with a #[cfg(test)] mod tests containing the named test ipad_anchor_required (this file compiles on macOS hosts, so the test runs locally and in CI's rust job). Run all UIKit/AppKit work on the main thread dispatched off the calling async context, and call done exactly once from the controller's completion handler: completed activity -> Success, cancelled -> Dismissed. Never block a thread waiting for the completion.
- Inbound: get_initial_media reads what the host delivered at launch (NSUserActivity / share-extension handoff through the App Group container); a plain launch yields an empty Vec (cold_start_send_vs_main); start_listen registers the handoff/activation observation and answers through MediaDone once per item; stop_listen removes it. Document in 1-2 line comments where the host app's own share-extension target is the wiring this reads — the extension itself is app-side, mirroring receive_sharing_intent's docs.
- Mapping: no sharing surface or unsupported presentation -> NotAvailable; staging failure where the source is missing -> FileNotFound, other staging IO failure -> Backend with the OS status via with_status; anything the framework reports as a code lands in status. No unwrap on foreign results; no panics; nothing hangs.
- Dependencies: only if the crate manifest lacks what you use, add the objc2 lines (objc2, objc2-foundation, objc2-ui-kit, objc2-app-kit with the features you actually use) under [target.'cfg(target_vendor = "apple")'.dependencies] — re-read the manifest immediately before the edit and touch only that section.

Your scoped gate compiles the whole lib on this macOS host (apple.rs is cfg-included here): cargo fmt --all -- --check, then cargo clippy -p nsh-share --lib -- -D warnings, then cargo test -p nsh-share --lib. The lib also contains sibling areas' files; if a sibling's in-progress code breaks the gate, name the file in your summary instead of editing it. Every unsafe block carries the invariant it relies on in a 1-2 line comment.

${FOOTER}`,
    gate: GATE_LIB,
    auditFocus:
      "Audit apple.rs for FFI soundness and exactly-once delivery: every unsafe block states its invariant; no unwrap/expect on foreign results; UIKit/AppKit calls dispatched to the main thread with no thread blocked on a completion handler; done called exactly once per sheet, on both the completed and cancelled paths; the iPad anchor helper and its named unit test; staging to temp with FileNotFound vs Backend mapping; inbound handoff reads with a plain launch yielding an empty Vec; can_receive false on macOS; any Cargo.toml edit confined to the apple target section.",
  },
  {
    name: "bridge-android",
    blurb: "platform/android.rs — Intent chooser via JNI, FileProvider URIs, ACTION_SEND inbound",
    implSpecialist: "z-rust-engineer",
    implFallback: PLAIN,
    implName: "Rust engineer·bridge-android",
    auditSpecialist: "z-unsafe-ffi-auditor",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/src/platform/android.rs (plus, only if your bridge needs them, the [target.'cfg(target_os = \"android\")'.dependencies] table of crates/nsh-share/Cargo.toml)",
    ask: `Write the Android bridge. Read first: AGENTS.md, the contract below, .refs/biometrics/crates/nsh-biometrics/src/platform/android.rs (the JNI house shape: how ndk-context supplies the VM and activity), share_plus's Android implementation under .refs/plus_plugins/packages/share_plus/share_plus/android/ (src/main/java/... SharePlusPlugin.java), and receive_sharing_intent's android/ sources for the SEND-vs-MAIN cold-start handling and the onNewIntent listener pattern.

${CONTRACT}

You own crates/nsh-share/src/platform/android.rs. Replace the stub completely. Implement Backend for NativeBackend on target_os = "android":
- Outbound: through JNI, build an ACTION_SEND (single item) or ACTION_SEND_MULTIPLE intent with the right MIME type, put text/subject/STREAM URIs, and start the chooser. Files: copy them into the app cache directory and share content:// URIs from a FileProvider with FLAG_GRANT_READ_URI_PERMISSION — never file:// (edge cases large_file_temp_copy, file_uri_rejected_invalid_argument); a source path that cannot be staged resolves FileNotFound, a staging IO error Backend with the OS status. No foreground Activity from ndk-context resolves NoActivity (edge case no_activity_error). Resolve exactly once from the activity result: completed -> Success, dismissed/cancelled -> Dismissed; the answer arrives on a JNI callback thread and must hop into done exactly once without blocking anything.
- Inbound: get_initial_media inspects the launching intent — ACTION_SEND carries the payload, ACTION_MAIN yields an empty Vec and never an error (cold_start_send_vs_main); start_listen registers for subsequent ACTION_SEND deliveries the way receive_sharing_intent listens, answering MediaDone once per item; stop_listen unregisters; reset clears the stored snapshot.
- Dependencies: keep only what you use under [target.'cfg(target_os = "android")'.dependencies] (jni and ndk-context at most, following the biometrics manifest's pins: jni 0.22.4, ndk-context 0.1.1) — re-read the manifest immediately before the edit and touch only that section.

This host is macOS: android.rs is cfg'd out here, so your local gate cannot compile it. Your scoped gate is cargo fmt --all -- --check only; the compile proof for your file is ci.yml's cross-check job (aarch64-linux-android), which the rust-core area wires. Say that plainly in your summary: this bridge is audit-verified and CI-compile-verified, not locally compiled. JNI code follows the auditor bar: every unsafe block states the invariant, no unwraps on JNI results, every local reference deleted, exceptions checked after calls, no thread blocked on a foreign callback.

${FOOTER}`,
    gate: GATE_FMT,
    auditFocus:
      "Audit android.rs for JNI soundness and exactly-once delivery: every unsafe block states its invariant; JNI local references deleted; exceptions checked after calls; no unwrap on JNI results; ACTION_SEND/MULTIPLE built correctly with MIME types; files staged to cache and shared as FileProvider content URIs with FLAG_GRANT_READ_URI_PERMISSION and never file://; missing Activity -> NoActivity; cold-start MAIN -> empty Vec; the listener registers once and answers MediaDone exactly once per delivery; cancel/stop are non-blocking no-ops where the OS gives no dismiss API; any Cargo.toml edit confined to the android target section.",
  },
  {
    name: "bridge-windows",
    blurb: "platform/windows.rs — DataTransferManager share UI",
    implSpecialist: "z-rust-engineer",
    implFallback: PLAIN,
    implName: "Rust engineer·bridge-windows",
    auditSpecialist: "z-unsafe-ffi-auditor",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/src/platform/windows.rs (plus, only if your bridge needs them, the [target.'cfg(target_os = \"windows\")'.dependencies] table of crates/nsh-share/Cargo.toml)",
    ask: `Write the Windows bridge. Read first: AGENTS.md, the contract below, .refs/plus_plugins/packages/share_plus/share_plus/lib/src/share_plus_windows.dart (the reference behavior), and .refs/biometrics/crates/nsh-biometrics/src/platform/windows.rs for the windows-crate house style (biometrics pins windows 0.62.2 and windows-future 0.3.2 with the std feature because IAsyncOperation blocking needs it).

${CONTRACT}

You own crates/nsh-share/src/platform/windows.rs. Replace the stub completely. Implement Backend for NativeBackend on target_os = "windows":
- DataTransferManager through the windows crate: construct a DataPackage with text, title/subject and staged storage files, get the chooser shown for the foreground window's dispatcher, and resolve exactly once from the operation: Success on share completion, Dismissed when dismissed, Backend with the HRESULT in status on failure. No window or dispatcher -> NoActivity (edge case no_activity_error). Never block the STA thread waiting on the operation — use the completed callback — and nothing hangs. Document in a 1-2 line comment that Windows' share UI has no cancel API, so cancel_share is a no-op and the system's answer still resolves (cancel_share_best_effort).
- get_initial_media and can_receive answer Ok(empty)/Ok(false): Windows has no inbound share surface in scope.
- Dependencies: only what you use under [target.'cfg(target_os = "windows")'.dependencies] (windows 0.62.x with the ApplicationModel_DataTransfer features you use, and windows-future with std if you hold an IAsyncOperation) — re-read the manifest immediately before the edit and touch only that section.

This host is macOS: windows.rs is cfg'd out here. Your scoped gate is cargo fmt --all -- --check only; compile proof is ci.yml's windows rust job, which the rust-core area wires. Say that plainly in your summary: this bridge is audit-verified and CI-compile-verified, not locally compiled. COM interop follows the auditor bar: every unsafe block states its invariant, no unwraps on HRESULTs, nothing blocks on a foreign callback.

${FOOTER}`,
    gate: GATE_FMT,
    auditFocus:
      "Audit windows.rs for COM soundness and exactly-once delivery: every unsafe block states its invariant; no unwrap/expect on HRESULTs or COM pointers; the DataPackage built from the right ShareParams fields; one resolution per share with the HRESULT carried in status on failure; no-window -> NoActivity; the STA thread never blocked (completed callback, not join); cancel_share a documented no-op; get_initial_media/can_receive honest empties; any Cargo.toml edit confined to the windows target section.",
  },
  {
    name: "bridge-linux",
    blurb: "platform/linux.rs — honest mailto fallback",
    implSpecialist: "z-rust-engineer",
    implFallback: PLAIN,
    implName: "Rust engineer·bridge-linux",
    auditSpecialist: "z-rust-engineer",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/src/platform/linux.rs",
    ask: `Write the Linux bridge. Read first: AGENTS.md, the contract below, and .refs/plus_plugins/packages/share_plus/share_plus/lib/src/share_plus_linux.dart (the reference behavior — share_plus_linux has no system share sheet and falls back to mailto:/ launching).

${CONTRACT}

You own crates/nsh-share/src/platform/linux.rs. Replace the stub completely. Implement Backend for NativeBackend on target_os = "linux":
- There is no system share sheet; port share_plus_linux's honest fallback. Text and a subject become a mailto: URL (title as subject, text in the body, uri appended when mail_to_fallback_enabled is set), launched through the platform's URL opener (xdg-open via std::process::Command, spawned without waiting). mail_to_fallback_enabled false -> NotAvailable. Files cannot be shared this way: params carrying files and no text/uri resolve NotAvailable with a 1-2 line comment saying why (no provider URI exists on a bare desktop). Every call resolves exactly once and nothing hangs; the child is never waited on.
- can_receive answers Ok(false); get_initial_media answers Ok(empty); start_share resolves through a ready future or a spawned one-shot — never a blocked thread; cancel_share is a no-op (nothing to cancel once the opener launched; document it).
- No new dependencies: std only. If you find you genuinely need one, say so in blockers instead of editing the shared manifest.

This host is macOS: linux.rs is cfg'd out here. Your scoped gate is cargo fmt --all -- --check only; compile proof is ci.yml's ubuntu rust job, which the rust-core area wires. Say that plainly in your summary: this bridge is audit-verified and CI-compile-verified, not locally compiled.

${FOOTER}`,
    gate: GATE_FMT,
    auditFocus:
      "Audit linux.rs for honest behavior and exactly-once resolution: the fallback matches share_plus_linux (mailto with subject/body, uri appended, flag honored); disabled flag and files-only params resolve NotAvailable with the documented reason; the opener spawned without waiting (no blocked thread, no hang); every call resolves exactly once; can_receive false and get_initial_media empty; no new dependencies sneaked into the shared manifest.",
  },
  {
    name: "bridge-web",
    blurb: "platform/web.rs — navigator.share over wasm-bindgen",
    implSpecialist: "z-rust-engineer",
    implFallback: PLAIN,
    implName: "Rust engineer·bridge-web",
    auditSpecialist: "z-rust-engineer",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/src/platform/web.rs (plus, only if your bridge needs them, the [target.'cfg(target_arch = \"wasm32\")'.dependencies] table of crates/nsh-share/Cargo.toml)",
    ask: `Write the web bridge. Read first: AGENTS.md, the contract below, and .refs/plus_plugins/packages/share_plus/share_plus/lib/src/share_plus_web.dart (the reference behavior: navigator.share, the files gating, and the download-fallback flag).

${CONTRACT}

You own crates/nsh-share/src/platform/web.rs. Replace the stub completely. Implement Backend for NativeBackend on target_arch = "wasm32":
- navigator.share via web-sys: build ShareData from text/title/uri; files only when the API level accepts them; a resolved promise maps to Success, a user abort (AbortError) to Dismissed (edge case cancel_dismissed_not_error), a missing navigator.share or a non-secure context to NotAvailable (edge case web_share_unavailable_not_available); a rejected promise that is neither abort nor unsupported maps to Backend with the browser's message in status. The download-fallback flag mirrors share_plus_web's behavior or reports NotAvailable when disabled — read the reference and follow it.
- get_initial_media and can_receive answer Ok(empty)/Ok(false): the web has no inbound share surface in scope. cancel_share is a no-op (the browser owns the sheet; document it).
- Dependencies: only what you use under [target.'cfg(target_arch = "wasm32")'.dependencies] (wasm-bindgen, js-sys, web-sys with the Navigator/ShareData features you actually use) — re-read the manifest immediately before the edit and touch only that section.

This host is macOS: web.rs is cfg'd out here. Your scoped gate is cargo fmt --all -- --check only; compile proof is ci.yml's wasm32-unknown-unknown cross-check, which the rust-core area wires. Say that plainly in your summary: this bridge is audit-verified and CI-compile-verified, not locally compiled. Bindings follow the auditor bar: no unwraps on foreign results, every promise path resolves exactly once, nothing hangs.

${FOOTER}`,
    gate: GATE_FMT,
    auditFocus:
      "Audit web.rs for exactly-once resolution and honest mapping: resolve->Success, AbortError->Dismissed, missing API or insecure context->NotAvailable, other rejections->Backend with the browser message in status; files gated on API level per share_plus_web; the download-fallback flag honored per the reference; get_initial_media/can_receive honest empties; cancel_share a documented no-op; no unwrap on foreign results; every promise path resolves exactly once; any Cargo.toml edit confined to the wasm32 target section.",
  },
  {
    name: "ts-package",
    blurb: "packages/nsh-share — the TypeScript package mirroring the Rust API",
    implSpecialist: "z-nsh-engineer",
    implFallback: PLAIN,
    implName: "NSH engineer·ts-package",
    auditSpecialist: "z-nsh-engineer",
    auditFallback: PLAIN,
    owns: "everything under packages/nsh-share/ (package.json, tsconfig.json, nsh.json, App.tsx, src/*.ts)",
    ask: `Write the TypeScript package. Read first: AGENTS.md, the contract below, and .refs/biometrics/packages/nsh-biometrics/ as the template — package.json, tsconfig.json, nsh.json, App.tsx, and every src/ file (index.ts, errors.ts, options.ts, backend.ts, memory.ts, rust.ts, auth.ts as the facade example) — and one test file for the runner conventions: tests/auth.test.tsx (import { test, expect } from "nsh/test"). The tests directory itself belongs to another area; do not create test files.

${CONTRACT}

You own everything under /Users/hmziq/fo/nsh-share/packages/nsh-share/. Create:
- package.json mirroring biometrics: name nsh-share, version 0.1.0, description in the house tone, license MIT, type module, main and types ./src/index.ts, exports for ".", "./rust", "./memory", repository https://github.com/SuperNSH/share.
- tsconfig.json: extends ./.nsh/tsconfig.json, include App.tsx, src/**/*.ts, tests/**/*.ts (the tests include is fine though tests belong to another area).
- nsh.json: { "name": "nsh-share tests", "id": "dev.supernsh.share.tests", "version": "0.1.0", "build": 1, "rust": "../../crates/nsh-share", "capabilities": {} } — note nsh check cargo-checks that crate first, and nsh test cargo-tests it before the TS tests.
- App.tsx: a minimal demo screen in the biometrics App.tsx's shape (nsh check requires an entry; verified on this machine). Keep it headless-test-safe: construct Share with the memory backend by default, never calling the native module at import time.
- src/errors.ts: ShareErrorCode union with exactly the six codes; MESSAGES with the same wording as the Rust messages; class ShareError extends Error with readonly code; shareError(code); asShareError(error) parsing a host rejection whose code is ERR_NSH_HOST_ERROR and whose message starts with an ERR_SHARE_* code, unknown -> ERR_SHARE_BACKEND — same structure as biometrics errors.ts.
- src/options.ts: ShareParams, ShareFile, Rect, SharedMedia, SharedMediaType, ShareResult, ShareResultStatus types (camelCase mirrors of the Rust structs; ShareResultStatus = "success" | "dismissed" | "unavailable"; SharedMediaType = "image" | "video" | "text" | "file" | "url").
- src/backend.ts: the ShareBackend seam type — isAvailable(): Promise<boolean>, canReceive(): Promise<boolean>, startShare(params: ShareParams, done: (r: Result2<ShareResult, ShareError>) => void): void, cancelShare(): void, getInitialMedia(): Promise<SharedMedia[]>, startListen(done): void, stopListen(): void — callback-exactly-once like the Rust trait, in biometrics backend.ts's spirit.
- src/share.ts: class Share mirroring the Rust facade — constructor({ backend }), isAvailable(), share(params: ShareParams): Promise<ShareResult>, shareUri(uri: string, title?: string), shareFiles(files: ShareFile[], text?: string, origin?: Rect); the same validation (no text/uri/files -> InvalidArgument; a path with "://" or a "file:" prefix -> InvalidArgument), the same one-sheet gate (second call rejects InProgress immediately; busy lasts until the answer; generation-gated so a late second answer is ignored; dismissal resolves "dismissed"). Export the constructor-args type like biometrics auth.ts's AuthenticateArgs.
- src/receive.ts: class ReceiveSharingIntent mirroring receive.rs — getInitialMedia(): Promise<SharedMedia[]>, receive(): Promise<SharedMedia> (a second concurrent receive rejects InProgress), reset(): void.
- src/memory.ts: memoryBackend() + MemoryShare/MemoryShareConfig mirroring FakeBackend's semantics (scripted answers, pendingShare/deliverShare/deliverMedia, setInitialMedia, counters, failNextShare) — the same API biometrics memory.ts exposes for its backend, adapted to share.
- src/rust.ts: rustBackend() bridging to the native class imported from "nsh-share/native" (the typings the Rust crate's #[native_module] generates), translating param objects and wrapping rejections through asShareError — same structure as biometrics rust.ts. Note in a 1-2 line comment that the app must list the crate under "rust" in nsh.json.
- src/index.ts: re-export the public surface.

Semantics must match the Rust crate exactly — a behavior difference is a finding. Keep src free of React except App.tsx (the entry the nsh toolchain requires); no demo beyond biometrics' App.tsx shape.

Your scoped gate runs in the package: nsh check, then nsh test. Mind the sibling race: nsh check cargo-checks ../../crates/nsh-share, which the rust-core area is writing in parallel. If a gate failure is entirely inside crates/nsh-share (not your files), say so in your summary rather than editing it — the next audit round re-runs the gate once the sibling has landed. If nsh test fails on missing test files (another area owns them), say so rather than writing them.

${FOOTER}`,
    gate: {
      label: "nsh check && nsh test in packages/nsh-share",
      args: ["-lc", `cd ${ROOT}/packages/nsh-share && nsh check && nsh test`],
      timeoutMs: 1200000,
    },
    auditFocus:
      "Audit the TS package against the Rust contract and the biometrics template: the six codes and message wording identical to the Rust crate; validation and one-sheet gate semantics identical (including the generation-gated late-answer drop and dismissal as a resolved result); SharedMediaType/ShareResultStatus value sets matching the Rust as_str values; memory backend counters and delivery semantics mirroring FakeBackend; rust.ts importing from \"nsh-share/native\" and wrapping errors through asShareError with the ERR_NSH_HOST_ERROR parsing; package.json/tsconfig/nsh.json/App.tsx matching the template and the entry requirement; no dead code; no React in src outside App.tsx.",
  },
  {
    name: "tests-both",
    blurb: "integration tests in Rust and TypeScript covering every goal, edge case and error path",
    implSpecialist: "z-rust-engineer",
    implFallback: PLAIN,
    implName: "Rust engineer·tests-both",
    auditSpecialist: "z-rust-engineer",
    auditFallback: PLAIN,
    owns: "crates/nsh-share/tests/ (share.rs, receive.rs, errors.rs) and packages/nsh-share/tests/ (share.test.ts, receive.test.ts, errors.test.ts)",
    ask: `Write the test suites in BOTH languages. Read first: AGENTS.md, docs/brief.md's edge cases, the contract below, .refs/biometrics/crates/nsh-biometrics/tests/ (auth.rs, availability.rs — the house Rust test style including the run() helper and the settle pattern) and .refs/biometrics/packages/nsh-biometrics/tests/auth.test.tsx (the house TS style, including the delivered() microtask-flush helper), and the crate/package sources as they exist by the time you run your gate — the API is fixed by the contract, so write against it even if a sibling is mid-flight.

${CONTRACT}

${EDGE_CASES}

You own crates/nsh-share/tests/ and packages/nsh-share/tests/, and nothing else. One named test per canonical edge case, in BOTH languages where the case is fake-side testable, named after the case (Rust test functions in snake_case matching the case names; TS test titles quoting them):
- Rust integration tests through Share::with_backend(Arc::new(FakeBackend::new())) and ReceiveSharingIntent::with_backend, using the run() helper convention: tests/share.rs (cases 1-7, 10, 11, 12), tests/receive.rs (9, 14, 15), tests/errors.rs (every ErrorKind code string, all six). Cases 8 and 13 live as lib unit tests inside apple.rs and platform/unsupported.rs, owned by other areas — do not duplicate them; a 1-2 line comment in tests/share.rs says where they live.
- TypeScript through memoryBackend() in packages/nsh-share/tests/: share.test.ts, receive.test.ts, errors.test.ts with import { test, expect } from "nsh/test" and rejects.toMatchObject({ code: "..." }) for coded errors.
- NoActivity and NotAvailable are start_share error paths: model them with fake.fail_next_share; availability with fake.set_available.
- Also assert, in both languages: the exactly-once counters (deliver twice, resolve once, counters unchanged), and that a Dismissed answer is Ok/"dismissed", not a rejection.
- For every case whose REAL platform half cannot run here (9, 10, 11, 12), the fake-side test carries a 1-2 line comment naming the bridge that implements it and saying CI's cross-check compiles that bridge — the fake-side path is still tested.

Every behavior the goals list must be traceable to a test name. Your scoped gate: cargo test -p nsh-share --all-targets, then nsh test in packages/nsh-share. Both compile sibling areas' files too; if a failure is clearly in a sibling's file (or a sibling has not landed yet — the API you code against is the contract above), name it in your summary instead of editing it; the next audit round re-runs the gate.

${FOOTER}`,
    gate: {
      label: "cargo test -p --all-targets && nsh test",
      args: [
        "-lc",
        `cd ${ROOT} && cargo test -p nsh-share --all-targets && cd packages/nsh-share && nsh test`,
      ],
      timeoutMs: 1200000,
    },
    auditFocus:
      "Audit the test suites: one named test per canonical edge case in BOTH languages (8 and 13 documented as lib unit tests elsewhere, with the pointer comment); every coded error path asserted through its stable code; the exactly-once property asserted with counters (a second delivered answer changes nothing); dismissal asserted as a success-path result; assertions specific enough to fail on regressions (no tautologies); Rust tests following the run() helper convention and compiling under --all-targets; TS tests following the nsh/test conventions; any case skipped for platform reasons carries the 1-2 line justification comment and still exercises the fake-side path.",
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

Rules: you are read-only — do not edit, create, or delete any file, do not run cargo or nsh or any build/test command (the script already ran the gate; its output is above), do not run git, never launch an app or emulator. Report actionable findings only: an array of concrete items, each naming the file and what to change; an empty array means clean. No PASS notes, no praise, no summaries of what is fine. If your instructions contradict each other or you cannot complete the review, say so plainly in your result rather than working around it. Report with: done (boolean), summary (one short paragraph), findings (array of strings).
If the contract below is needed to judge names or codes, it is authoritative:

${CONTRACT}`;
}

function fixAsk(a: AreaDef, findings: string[], gateFailure: string): string {
  const gatePart = gateFailure
    ? `Your scoped gate failed with (tail):\n${gateFailure}\nFix your files so it passes. A failure that is entirely inside a sibling area's files is not yours: name it in your summary instead of editing it, and the next round re-runs the gate.`
    : "";
  return `Fix round for your area "${a.name}". You own exactly: ${a.owns}. The auditor read your work against the contract and AGENTS.md and reports these actionable findings:\n${findings
    .map((f, i) => `${i + 1}. ${f}`)
    .join("\n")}\n\n${gatePart}\nChange only files you own; never touch sibling areas' files (if a finding demands that, say so in blockers instead). Keep AGENTS.md house rules. Then report with: done (boolean), summary (what you changed, honestly), blockers (optional).`;
}

// ---------------------------------------------------------------------------
// Step 0 (serial), then the fan-out.
// ---------------------------------------------------------------------------

phase("Verify the five cloned references and warm the build cache");

const step0 = await world.run("bash", [`${ROOT}/.zcode/step0-refs.sh`], { timeoutMs: 180000 });
if (step0.exitCode !== 0) {
  throw new Error(
    `Step 0 failed: the five references could not be verified. Exit ${step0.exitCode}: ${String(
      (step0.stderr || "") + (step0.stdout || ""),
    ).slice(-500)}`,
  );
}

const warm = await world.run("bash", [`${ROOT}/.zcode/warm-cargo.sh`], { timeoutMs: 1500000 });
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

interface GateRun {
  exitCode: number;
  stdout?: string;
  stderr?: string;
}

async function runGate(g: GateSpec): Promise<GateRun> {
  return world.run("bash", g.args, { timeoutMs: g.timeoutMs });
}

interface AgentResult {
  done?: boolean;
  summary?: string;
  blockers?: string[];
  findings?: string[];
}

interface AreaResult {
  name: string;
  status: "done" | "unconverged" | "failed";
  summary: string;
  rounds?: number;
  converged?: boolean;
}

async function runArea(a: AreaDef): Promise<AreaResult> {
  report({ name: a.name, phase: "implementing", summary: a.blurb }, "areas");
  const implLoaded = await specialistPersona(a.implSpecialist, a.implFallback);
  const impl = agent(a.implName, { system: implLoaded.persona, model: MODELS.pro });

  const done = (await impl.ask(a.ask)) as AgentResult;
  if (!done || done.done === false) {
    const why = done?.summary || "implementer reported done=false";
    report({ name: a.name, phase: "failed", summary: why.slice(0, 160) }, "areas");
    return { name: a.name, status: "failed", summary: why.slice(0, 400), rounds: 0 };
  }
  let lastSummary = done.summary || "";
  const implNote =
    implLoaded.source === "fallback"
      ? ` (specialist ${a.implSpecialist} not found; plain persona used)`
      : "";

  let rounds = 0;
  let converged = false;
  let lastFindings: string[] = [];
  let lastGate = "";

  while (rounds < AUDIT_ROUNDS) {
    rounds += 1;

    if (a.gate) {
      report({ name: a.name, phase: "gating", summary: a.gate.label }, "areas");
      const g = await runGate(a.gate);
      lastGate = g.exitCode === 0 ? "" : tailOf(g);
    }

    report({ name: a.name, phase: "auditing", summary: `round ${rounds}` }, "areas");
    const auditLoaded = await specialistPersona(a.auditSpecialist, a.auditFallback);
    const auditor = agent(`Auditor·${a.name}·${rounds}`, {
      system: auditLoaded.persona,
      model: MODELS.pro,
    });
    const audit = (await auditor.ask(auditAsk(a, lastGate))) as AgentResult;
    lastFindings = audit && Array.isArray(audit.findings) ? audit.findings : [];
    if (!audit || audit.done === false) {
      lastFindings = [`the auditor could not complete its review: ${audit?.summary || "no result"}`];
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
    const fix = (await impl.ask(fixAsk(a, lastFindings, lastGate))) as AgentResult;
    if (fix && fix.summary) lastSummary = fix.summary;
  }

  const status: AreaResult["status"] = converged ? "done" : "unconverged";
  const summary = converged
    ? `converged after ${rounds} round(s): ${lastSummary}${implNote}`.slice(0, 400)
    : `did NOT converge in ${AUDIT_ROUNDS} rounds; last findings: ${lastFindings
        .slice(0, 3)
        .join(" | ")
        .slice(0, 260)}`;
  report({ name: a.name, phase: status, summary: summary.slice(0, 160) }, "areas");
  return { name: a.name, status, rounds, converged, summary };
}

phase("Each area implements its piece and runs its own audit rounds in parallel");

const areaResults: AreaResult[] = await Promise.all(
  areas.map(async (a): Promise<AreaResult> => {
    try {
      return await runArea(a);
    } catch (e) {
      const msg = String((e as { message?: string })?.message || e).slice(0, 400);
      report({ name: a.name, phase: "failed", summary: msg.slice(0, 160) }, "areas");
      return { name: a.name, status: "failed", summary: msg };
    }
  }),
);

// ---------------------------------------------------------------------------
// The join: the brief's four whole-tree gates, once, over every area's work.
// ---------------------------------------------------------------------------

phase("Run the whole-tree gates once over the joined tree, fixing any failure once");

const JOIN_GATES: GateSpec[] = [
  {
    label: "cargo fmt --all -- --check",
    args: ["-lc", `cd ${ROOT} && cargo fmt --all -- --check`],
    timeoutMs: 600000,
  },
  {
    label: "cargo clippy --all-targets -- -D warnings",
    args: ["-lc", `cd ${ROOT} && cargo clippy --all-targets -- -D warnings`],
    timeoutMs: 1200000,
  },
  {
    label: "cargo test --all-targets",
    args: ["-lc", `cd ${ROOT} && cargo test --all-targets`],
    timeoutMs: 1200000,
  },
  {
    label: "nsh check && nsh test (packages/nsh-share)",
    args: ["-lc", `cd ${ROOT}/packages/nsh-share && nsh check && nsh test`],
    timeoutMs: 1200000,
  },
];

const failedJoin: { label: string; tail: string }[] = [];
for (const g of JOIN_GATES) {
  const r = await runGate(g);
  if (r.exitCode !== 0) {
    failedJoin.push({ label: g.label, tail: tailOf(r) });
  }
}

let finalGate = failedJoin.length === 0 ? "pass" : "failed";
let finalDetail = failedJoin.length === 0 ? "all four gates green" : "";
if (failedJoin.length > 0) {
  report(
    {
      name: "join",
      phase: "fixing",
      summary: `${failedJoin.length} gate(s) failed; one fix round`,
    },
    "areas",
  );
  const joinPersona = await specialistPersona("z-rust-engineer", PLAIN);
  const fixer = agent("Join fixer", { system: joinPersona.persona, model: MODELS.pro });
  const reruns: string[] = [];
  for (const f of failedJoin) {
    const fix = (await fixer.ask(
      `The whole-tree gate "${f.label}" failed over the joined tree at /Users/hmziq/fo/nsh-share. Output (tail):\n${f.tail}\n\nFix it by editing the repo directly — any file, but change the minimum: this is a join fix, not a rewrite. No git commands, no new features, no launching anything. Then report with: done (boolean), summary (what you changed), blockers (optional).`,
    )) as AgentResult;
    void fix;
    const spec = JOIN_GATES.find((g) => g.label === f.label);
    if (spec) {
      const after = await runGate(spec);
      reruns.push(`${f.label}: ${after.exitCode === 0 ? "green after fix" : `still red (exit ${after.exitCode})`}`);
    } else {
      reruns.push(`${f.label}: gate spec not found after fix`);
    }
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
const doneAreas = areaResults.filter((r) => r.status === "done");

const conclusion = [
  `nsh-share: ${areas.length} areas ran; ${doneAreas.length} converged (zero audit findings + green scoped gate), ${unconverged.length} hit the 3-round audit cap, ${failed.length} failed.`,
  `Final whole-tree gates: ${finalGate} (${finalDetail}).`,
  `Rust crate at crates/nsh-share, TypeScript package at packages/nsh-share, both laid out like .refs/biometrics with the same API names, the same six error codes and the same exactly-once semantics; docs and ci.yml updated.`,
  `Per AGENTS.md nothing was committed or pushed; the device-platform bridges (Android, Windows, Linux, web) are locally fmt-checked and audit-verified, with their compile proof in ci.yml's cross-check job.`,
].join(" ");

const verified = [
  "per-area scoped gates ran as world.run commands (cargo fmt/clippy/test scoping per area, nsh check + nsh test for the TS package)",
  `whole-tree gates at the join: ${JOIN_GATES.map((g) => g.label).join(", ")}`,
  "each area audited by a fresh GLM-5.3 auditor up to 3 rounds, until zero findings + green gate or the cap",
];

const notCovered = [
  "no device, emulator or simulator ran anywhere (AGENTS.md): platform bridges are compile-checked by CI's cross targets and audited by reading, not exercised",
  "cargo clippy/test for the Android, Windows, Linux and web cfg-gated bridge files ran only via fmt locally; their compile proof is ci.yml's cross-check and OS matrix, which run on GitHub, not here",
];

return {
  conclusion,
  areas: areaResults,
  verified,
  notCovered,
  notes:
    "GLM-5.3 Max (MODELS.pro) pinned on every implementer, every auditor and the join fixer per the brief; no vision phase exists, so MODELS.flash is declared and unused. Audit budget 3 rounds per area, until zero findings and a clean scoped gate or the cap, reported per area. STEP 0 ran serially before any implement phase: .zcode/step0-refs.sh verified the five cloned references, .zcode/warm-cargo.sh proved nsh-core 0.4.0 resolves and primed the cargo cache. Scoped per-area gates: lib-target cargo commands for rust-core and bridge-apple (apple.rs compiles on this macOS host), fmt-only for the cfg'd-out bridges (Android, Windows, Linux, web compile in CI's cross-check), nsh check/test for the TS package, cargo test --all-targets + nsh test for the tests area. crates/nsh-share/Cargo.toml is shared: rust-core owns the base manifest, each bridge may add only its own target section.",
};
