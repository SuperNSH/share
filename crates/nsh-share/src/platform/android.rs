//! Android: the chooser over JNI, and inbound `ACTION_SEND` from the launching intent.
//!
//! `ndk-context` supplies the VM and the context, and the context must itself be the foreground
//! `Activity` (a host such as tao installs the activity; `nsh-biometrics` installs only the
//! application): a share needs a window to start from, and anything else answers `NoActivity`.
//! Files stage into the app's cache directory and share as `content:` URIs from NSH's own
//! `com.longbridge.nsh.FilesProvider`, which `nsh package` declares in every packaged manifest;
//! a `file:` URI would not leave the app.
//!
//! The chooser's outcome (completed or dismissed) reaches no pure-JNI bridge: NSH's activity
//! hands `onActivityResult` to no one, and a receiver for the chooser's `IntentSender` would be
//! app-side Java this crate cannot ship. So the answer is `Unavailable` the moment the system
//! took the sheet — `share_plus`'s own result-unavailable path — and `cancel_share` is a no-op:
//! only the system can dismiss another activity's chooser.
//!
//! Inbound: the launching intent is read for the cold start (`ACTION_SEND` carries the payload,
//! `ACTION_MAIN` answers empty). Later deliveries are seen by watching the activity's intent,
//! which is how a fresh activity instance arrives; an activity's `onNewIntent` never calls
//! `setIntent`, so a `singleTask` app (one with URL schemes) delivers only its cold start.

use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use jni::{
    Env, JValue, JValueOwned, JavaVM, jni_sig, jni_str,
    objects::{JObject, JString},
    refs::Reference as _,
    signature::MethodSignature,
    strings::JNIStr,
    sys::{jint, jobject},
};

use crate::{
    backend::{Backend, BackendFuture, MediaDone, ShareDone},
    error::{ErrorKind, ShareError},
    options::ShareParams,
    receive::{SharedMedia, SharedMediaType},
    share::{ShareResult, ShareResultStatus},
};

pub(crate) struct NativeBackend;

/// `Intent` names, by the string values the framework's constants hold.
const ACTION_SEND: &str = "android.intent.action.SEND";
const ACTION_SEND_MULTIPLE: &str = "android.intent.action.SEND_MULTIPLE";
const EXTRA_TEXT: &str = "android.intent.extra.TEXT";
const EXTRA_SUBJECT: &str = "android.intent.extra.SUBJECT";
const EXTRA_TITLE: &str = "android.intent.extra.TITLE";
const EXTRA_STREAM: &str = "android.intent.extra.STREAM";
/// `Intent.FLAG_GRANT_READ_URI_PERMISSION`.
const FLAG_GRANT_READ: jint = 1;
/// `MediaMetadataRetriever.OPTION_CLOSEST_SYNC`, the flag `receive_sharing_intent` frames with.
const CLOSEST_SYNC: jint = 2;
/// `MediaMetadataRetriever.METADATA_KEY_DURATION`.
const METADATA_DURATION: jint = 9;
/// The first Android with `getScaledFrameAtTime`; below it the full frame is taken.
const SDK_SCALED_FRAME: jint = 27;
/// Where this crate stages, under the app's own cache root: outbound copies and inbound
/// receives live apart, so an outbound share never clears what a receive handed the app.
const STAGE_OUT: &str = "nsh-share/share";
const STAGE_IN: &str = "nsh-share/receive";

impl From<jni::errors::Error> for ShareError {
    fn from(_: jni::errors::Error) -> Self {
        ShareError::new(ErrorKind::Backend)
    }
}

/// The VM and the context `ndk-context` holds, read once. `ndk_context::android_context` panics
/// while nothing is installed, so the probe is guarded the way `nsh-biometrics` guards it.
#[derive(Clone, Copy)]
struct Ptrs {
    vm: *mut jni::sys::JavaVM,
    context: jobject,
}

// SAFETY: the VM lives as long as the process, and ndk-context holds the context reference for
// just as long; both are read-only values that stay valid behind these pointers for any thread
// that reads them, which is what the static's Sync and Send need.
unsafe impl Send for Ptrs {}
unsafe impl Sync for Ptrs {}

static ANDROID: OnceLock<Option<Ptrs>> = OnceLock::new();

fn pointers() -> Option<Ptrs> {
    ANDROID
        .get_or_init(|| {
            // The getter panics on a fresh process; a silent hook keeps that out of the log.
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let probe = std::panic::catch_unwind(ndk_context::android_context);
            std::panic::set_hook(previous);
            probe.ok().map(|context| Ptrs {
                vm: context.vm().cast(),
                context: context.context().cast(),
            })
        })
        .clone()
}

// --- JNI plumbing ------------------------------------------------------------------
//
// Every fallible call below goes through one of these: a call whose Java exception is cleared
// the moment the call fails, so the next call never runs with one pending. `soft` answers
// `None` instead of an error, for the inbound parse, where a failed call drops one item and
// never the whole delivery — the way `receive_sharing_intent` catches each media read.

fn call<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'_>,
    name: &JNIStr,
    sig: MethodSignature<'static, 'static>,
    args: &[JValue],
) -> Result<JValueOwned<'local>, ShareError> {
    match env.call_method(obj, name, sig, args) {
        Ok(value) => Ok(value),
        Err(_) => {
            env.exception_clear();
            Err(ShareError::new(ErrorKind::Backend))
        }
    }
}

fn static_call<'local>(
    env: &mut Env<'local>,
    class: &JNIStr,
    name: &JNIStr,
    sig: MethodSignature<'static, 'static>,
    args: &[JValue],
) -> Result<JValueOwned<'local>, ShareError> {
    match env.call_static_method(class, name, sig, args) {
        Ok(value) => Ok(value),
        Err(_) => {
            env.exception_clear();
            Err(ShareError::new(ErrorKind::Backend))
        }
    }
}

fn new_object<'local>(
    env: &mut Env<'local>,
    class: &JNIStr,
    sig: MethodSignature<'static, 'static>,
    args: &[JValue],
) -> Result<JObject<'local>, ShareError> {
    match env.new_object(class, sig, args) {
        Ok(value) => Ok(value),
        Err(_) => {
            env.exception_clear();
            Err(ShareError::new(ErrorKind::Backend))
        }
    }
}

fn java_string<'local>(env: &mut Env<'local>, text: &str) -> Result<JObject<'local>, ShareError> {
    match env.new_string(text) {
        Ok(value) => Ok(value.into()),
        Err(_) => {
            env.exception_clear();
            Err(ShareError::new(ErrorKind::Backend))
        }
    }
}

/// Reads a Java `String`. Takes the reference by value: an owned local the caller will not
/// use again, which is every string this module reads.
fn read_string(env: &mut Env<'_>, text: JObject<'_>) -> Result<String, ShareError> {
    let text: JString<'_> = env.cast_local::<JString>(text).map_err(ShareError::from)?;
    match text.mutf8_chars(env) {
        Ok(chars) => Ok(String::from(chars)),
        Err(_) => {
            env.exception_clear();
            Err(ShareError::new(ErrorKind::Backend))
        }
    }
}

/// A method that answers a `String`, read strictly: `null` is a backend failure.
fn call_string(
    env: &mut Env<'_>,
    obj: &JObject<'_>,
    name: &JNIStr,
    sig: MethodSignature<'static, 'static>,
    args: &[JValue],
) -> Result<String, ShareError> {
    let value = call(env, obj, name, sig, args)?.l()?;
    if value.is_null() {
        return Err(ShareError::new(ErrorKind::Backend));
    }
    read_string(env, value)
}

fn soft<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'_>,
    name: &JNIStr,
    sig: MethodSignature<'static, 'static>,
    args: &[JValue],
) -> Option<JValueOwned<'local>> {
    match env.call_method(obj, name, sig, args) {
        Ok(value) => Some(value),
        Err(_) => {
            env.exception_clear();
            None
        }
    }
}

fn soft_static<'local>(
    env: &mut Env<'local>,
    class: &JNIStr,
    name: &JNIStr,
    sig: MethodSignature<'static, 'static>,
    args: &[JValue],
) -> Option<JValueOwned<'local>> {
    match env.call_static_method(class, name, sig, args) {
        Ok(value) => Some(value),
        Err(_) => {
            env.exception_clear();
            None
        }
    }
}

/// A method that answers an object, read softly; `null` and failures both answer `None`.
fn soft_object<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'_>,
    name: &JNIStr,
    sig: MethodSignature<'static, 'static>,
    args: &[JValue],
) -> Option<JObject<'local>> {
    let value = soft(env, obj, name, sig, args)?.l().ok()?;
    (!value.is_null()).then_some(value)
}

/// A method that answers a `String`, read softly; `null` and failures both answer `None`.
fn soft_string(
    env: &mut Env<'_>,
    obj: &JObject<'_>,
    name: &JNIStr,
    sig: MethodSignature<'static, 'static>,
    args: &[JValue],
) -> Option<String> {
    let value = soft_object(env, obj, name, sig, args)?;
    read_string_soft(env, value)
}

fn read_string_soft(env: &mut Env<'_>, text: JObject<'_>) -> Option<String> {
    let text: JString<'_> = env.cast_local::<JString>(text).ok()?;
    match text.mutf8_chars(env) {
        Ok(chars) => Some(String::from(chars)),
        Err(_) => {
            env.exception_clear();
            None
        }
    }
}

/// Whether the context is itself a foreground activity, which is what a share needs.
fn is_activity(env: &mut Env<'_>, context: &JObject<'_>) -> Result<bool, ShareError> {
    match env.is_instance_of(context, jni_str!("android/app/Activity")) {
        Ok(value) => Ok(value),
        Err(_) => {
            env.exception_clear();
            Err(ShareError::new(ErrorKind::Backend))
        }
    }
}

/// Runs `work` attached to the VM. The wrapper only borrows `ndk-context`'s pointers; neither it
/// nor the references it creates are deleted, which is what their owner assumes.
fn run_attached<R>(
    ptrs: Ptrs,
    work: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> Result<R, ShareError>,
) -> Result<R, ShareError> {
    // SAFETY: the VM pointer came from ndk-context, which keeps the VM for the whole process;
    // the wrapper only borrows it.
    let vm = unsafe { JavaVM::from_raw(ptrs.vm) };
    vm.attach_current_thread(|env| {
        // SAFETY: ndk-context holds this global context reference for the process, and the
        // wrapper only borrows it.
        let context = unsafe { JObject::from_raw(env, ptrs.context) };
        work(env, &context)
    })
}

/// Runs `work` off the engine thread, answering `none` when no context is installed at all.
async fn query<R: Send + 'static>(
    none: R,
    work: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> Result<R, ShareError> + Send + 'static,
) -> Result<R, ShareError> {
    let Some(ptrs) = pointers() else {
        return Ok(none);
    };
    nsh_core::prelude::blocking(move || run_attached(ptrs, work))
        .await
        .map_err(|_| ShareError::new(ErrorKind::Backend))
        .and_then(|answer| answer)
}

// --- Outbound ----------------------------------------------------------------------

/// What one share carries, as plain data; the worker thread owns it alone.
struct Plan {
    title: Option<String>,
    text: Option<String>,
    subject: Option<String>,
    uri: Option<String>,
    files: Vec<PlannedFile>,
}

struct PlannedFile {
    path: String,
    rename: Option<String>,
    mime: Option<String>,
}

fn plan(params: &ShareParams) -> Plan {
    Plan {
        title: params.title.clone(),
        text: params.text.clone(),
        subject: params.subject.clone(),
        uri: params.uri.clone(),
        files: params
            .files
            .iter()
            .enumerate()
            .map(|(at, file)| PlannedFile {
                path: file.path.clone(),
                rename: params.file_name_overrides.get(at).cloned(),
                mime: file.mime_type.clone(),
            })
            .collect(),
    }
}

/// The share's own file name: the override when there is one, the source's own name otherwise.
/// Only the final component of either is kept, so no name can point outside the staging folder.
fn staged_name(source: &Path, rename: Option<&str>) -> Option<String> {
    let name = rename.map_or(source, Path::new);
    let name = name.file_name()?.to_str()?;
    (!name.is_empty()).then_some(name.to_owned())
}

/// A staging failure with the OS's own code where it gave one; a missing source is a missing
/// file, everything else is the platform failing us.
fn io_error(error: std::io::Error) -> ShareError {
    let kind = if error.kind() == std::io::ErrorKind::NotFound {
        ErrorKind::FileNotFound
    } else {
        ErrorKind::Backend
    };
    let share_error = ShareError::new(kind);
    match error.raw_os_error() {
        Some(status) => share_error.with_status(i64::from(status)),
        None => share_error,
    }
}

/// Copies the listed files into `<cache>/nsh-share/share`, emptying it first the way
/// `share_plus` empties its share cache: nothing of an earlier share is left to hand out.
fn stage(cache: &Path, plan: &Plan) -> Result<Vec<PathBuf>, ShareError> {
    let dir = cache.join(STAGE_OUT);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(io_error)?;
    let mut names: HashSet<String> = HashSet::new();
    let mut staged = Vec::with_capacity(plan.files.len());
    for (at, file) in plan.files.iter().enumerate() {
        let source = Path::new(&file.path);
        let mut name = staged_name(source, file.rename.as_deref())
            .ok_or_else(|| ShareError::new(ErrorKind::FileNotFound))?;
        if !names.insert(name.clone()) {
            // Two files of one share keep apart when their names collide.
            name = format!("{at}-{name}");
            names.insert(name.clone());
        }
        let target = dir.join(&name);
        std::fs::copy(source, &target).map_err(io_error)?;
        staged.push(target);
    }
    Ok(staged)
}

/// Makes the thread's class loader the app's own, so class lookups reach `FilesProvider`: a
/// native thread's default loader does not see application classes.
fn adopt_app_loader(env: &mut Env<'_>, context: &JObject<'_>) -> Result<(), ShareError> {
    let class = call(
        env,
        context,
        jni_str!("getClass"),
        jni_sig!("()Ljava/lang/Class;"),
        &[],
    )?
    .l()?;
    let loader = call(
        env,
        &class,
        jni_str!("getClassLoader"),
        jni_sig!("()Ljava/lang/ClassLoader;"),
        &[],
    )?
    .l()?;
    if loader.is_null() {
        return Ok(());
    }
    let current = static_call(
        env,
        jni_str!("java/lang/Thread"),
        jni_str!("currentThread"),
        jni_sig!("()Ljava/lang/Thread;"),
        &[],
    )?
    .l()?;
    call(
        env,
        &current,
        jni_str!("setContextClassLoader"),
        jni_sig!("(Ljava/lang/ClassLoader;)V"),
        &[JValue::Object(&loader)],
    )?;
    Ok(())
}

/// The `content:` URI `FilesProvider` serves a staged file at, and the file's MIME type: the
/// caller's own when it named one, the provider's read of the extension otherwise.
fn provider_uri<'local>(
    env: &mut Env<'local>,
    context: &JObject<'_>,
    file: &Path,
    mime: Option<&str>,
) -> Result<(JObject<'local>, String), ShareError> {
    let path = java_string(env, &file.to_string_lossy())?;
    let file = new_object(
        env,
        jni_str!("java/io/File"),
        jni_sig!("(Ljava/lang/String;)V"),
        &[JValue::Object(&path)],
    )?;
    let uri = static_call(
        env,
        jni_str!("com/longbridge/nsh/FilesProvider"),
        jni_str!("uriFor"),
        jni_sig!("(Landroid/content/Context;Ljava/io/File;)Landroid/net/Uri;"),
        &[JValue::Object(context), JValue::Object(&file)],
    )?
    .l()?;
    if uri.is_null() {
        return Err(ShareError::new(ErrorKind::Backend));
    }
    let mime = match mime {
        Some(mime) => mime.to_owned(),
        None => {
            let guessed = static_call(
                env,
                jni_str!("com/longbridge/nsh/FilesProvider"),
                jni_str!("typeOf"),
                jni_sig!("(Landroid/content/Context;Ljava/io/File;)Ljava/lang/String;"),
                &[JValue::Object(context), JValue::Object(&file)],
            )?
            .l()?;
            if guessed.is_null() {
                "application/octet-stream".to_owned()
            } else {
                read_string(env, guessed)?
            }
        }
    };
    Ok((uri, mime))
}

/// One MIME type for many files: the one they share, their common `type/*`, or `*/*` —
/// `share_plus`'s reduction, which the chooser's own listing follows.
fn reduced_mime(mimes: &[String]) -> String {
    let base = |mime: &str| mime.split('/').next().unwrap_or("*");
    let mut common = mimes[0].clone();
    for mime in &mimes[1..] {
        if *mime == common {
            continue;
        }
        if base(mime) == base(&common) {
            common = format!("{}/*", base(mime));
        } else {
            return "*/*".to_owned();
        }
    }
    common
}

fn put_string_extra(
    env: &mut Env<'_>,
    intent: &JObject<'_>,
    key: &str,
    value: Option<&str>,
) -> Result<(), ShareError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let key = java_string(env, key)?;
    let value = java_string(env, value)?;
    call(
        env,
        intent,
        jni_str!("putExtra"),
        jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;"),
        &[JValue::Object(&key), JValue::Object(&value)],
    )?;
    Ok(())
}

/// Builds the share intent and starts the chooser from the context, which is the foreground
/// activity. Every Java object here is a local reference of the attach's frame.
fn show_sheet(
    env: &mut Env<'_>,
    context: &JObject<'_>,
    plan: &Plan,
    staged: &[PathBuf],
) -> Result<(), ShareError> {
    // The provider's class is the app's own: this thread must carry the app's class loader
    // before any `FilesProvider` call.
    adopt_app_loader(env, context)?;
    let mut mimes = Vec::with_capacity(staged.len());
    let mut uris = Vec::with_capacity(staged.len());
    for (at, file) in staged.iter().enumerate() {
        let (uri, mime) = provider_uri(env, context, file, plan.files[at].mime.as_deref())?;
        uris.push(uri);
        mimes.push(mime);
    }

    let intent = new_object(
        env,
        jni_str!("android/content/Intent"),
        jni_sig!("()V"),
        &[],
    )?;
    let action = java_string(
        env,
        if uris.len() > 1 {
            ACTION_SEND_MULTIPLE
        } else {
            ACTION_SEND
        },
    )?;
    call(
        env,
        &intent,
        jni_str!("setAction"),
        jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
        &[JValue::Object(&action)],
    )?;
    let mime = if uris.len() > 1 {
        reduced_mime(&mimes)
    } else if let [only] = &mimes[..] {
        only.clone()
    } else {
        // With nothing staged the share is text, and text is plain.
        "text/plain".to_owned()
    };
    let mime = java_string(env, &mime)?;
    call(
        env,
        &intent,
        jni_str!("setType"),
        jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
        &[JValue::Object(&mime)],
    )?;

    put_string_extra(env, &intent, EXTRA_SUBJECT, plan.subject.as_deref())?;
    put_string_extra(env, &intent, EXTRA_TITLE, plan.title.as_deref())?;
    if uris.is_empty() {
        // The URI is the share, as text; share_plus drops a text the URI would cover.
        put_string_extra(
            env,
            &intent,
            EXTRA_TEXT,
            plan.uri.as_deref().or(plan.text.as_deref()),
        )?;
    } else {
        put_string_extra(env, &intent, EXTRA_TEXT, plan.text.as_deref())?;
        let key = java_string(env, EXTRA_STREAM)?;
        if let [uri] = &uris[..] {
            call(
                env,
                &intent,
                jni_str!("putExtra"),
                jni_sig!("(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;"),
                &[JValue::Object(&key), JValue::Object(uri)],
            )?;
        } else {
            let list = new_object(env, jni_str!("java/util/ArrayList"), jni_sig!("()V"), &[])?;
            for uri in &uris {
                call(
                    env,
                    &list,
                    jni_str!("add"),
                    jni_sig!("(Ljava/lang/Object;)Z"),
                    &[JValue::Object(uri)],
                )?;
            }
            call(
                env,
                &intent,
                jni_str!("putParcelableArrayListExtra"),
                jni_sig!("(Ljava/lang/String;Ljava/util/ArrayList;)Landroid/content/Intent;"),
                &[JValue::Object(&key), JValue::Object(&list)],
            )?;
        }
        call(
            env,
            &intent,
            jni_str!("addFlags"),
            jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::Int(FLAG_GRANT_READ)],
        )?;
    }

    let title: JObject<'_> = match &plan.title {
        Some(title) => java_string(env, title)?,
        None => JObject::null(),
    };
    let chooser = static_call(
        env,
        jni_str!("android/content/Intent"),
        jni_str!("createChooser"),
        jni_sig!("(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;"),
        &[JValue::Object(&intent), JValue::Object(&title)],
    )?
    .l()?;
    let started = call(
        env,
        context,
        jni_str!("startActivity"),
        jni_sig!("(Landroid/content/Intent;)V"),
        &[JValue::Object(&chooser)],
    );
    if started.is_err() {
        // The chooser launch threw (nothing resolves the share, or the activity refused it):
        // no sheet is up.
        env.exception_clear();
        return Err(ShareError::new(ErrorKind::NotAvailable));
    }
    Ok(())
}

/// The whole outbound share, on its own thread: attach, stage, build, start, answer.
fn run_share(plan: Plan) -> Result<ShareResult, ShareError> {
    let Some(ptrs) = pointers() else {
        return Err(ShareError::new(ErrorKind::NoActivity));
    };
    run_attached(ptrs, |env, context| {
        if !is_activity(env, context)? {
            return Err(ShareError::new(ErrorKind::NoActivity));
        }
        let cache = call(
            env,
            context,
            jni_str!("getCacheDir"),
            jni_sig!("()Ljava/io/File;"),
            &[],
        )?
        .l()?;
        let cache = call_string(
            env,
            &cache,
            jni_str!("getAbsolutePath"),
            jni_sig!("()Ljava/lang/String;"),
            &[],
        )?;
        let staged = stage(Path::new(&cache), &plan)?;
        show_sheet(env, context, &plan, &staged)?;
        Ok(ShareResult {
            raw: "android/no-activity-result".to_owned(),
            status: ShareResultStatus::Unavailable,
        })
    })
}

// --- Inbound -----------------------------------------------------------------------

/// The type a MIME names, the way `receive_sharing_intent` reads it: the prefix before the
/// slash; no MIME means a plain file.
fn kind_of(mime: Option<&str>) -> SharedMediaType {
    match mime {
        Some(mime) if mime.starts_with("image") => SharedMediaType::Image,
        Some(mime) if mime.starts_with("video") => SharedMediaType::Video,
        Some(mime) if mime.starts_with("text") => SharedMediaType::Text,
        _ => SharedMediaType::File,
    }
}

/// The name a content resolver gives the URI, `_display_name` when the provider exposes it and
/// a generated one otherwise — `receive_sharing_intent`'s own fallback.
fn display_name(
    env: &mut Env<'_>,
    resolver: &JObject<'_>,
    uri: &JObject<'_>,
    mime: Option<&str>,
) -> Option<String> {
    let none = JObject::null();
    let cursor = soft_object(
        env,
        resolver,
        jni_str!("query"),
        jni_sig!(
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"
        ),
        &[
            JValue::Object(uri),
            JValue::Object(&none),
            JValue::Object(&none),
            JValue::Object(&none),
            JValue::Object(&none),
        ],
    );
    let from_cursor = (|| {
        let cursor = cursor.as_ref()?;
        let moved = soft(env, cursor, jni_str!("moveToFirst"), jni_sig!("()Z"), &[])
            .and_then(|moved| moved.z().ok())?;
        if !moved {
            return None;
        }
        let key = java_string_soft(env, "_display_name")?;
        let column = soft(
            env,
            cursor,
            jni_str!("getColumnIndex"),
            jni_sig!("(Ljava/lang/String;)I"),
            &[JValue::Object(&key)],
        )
        .and_then(|column| column.i().ok())?;
        if column < 0 {
            return None;
        }
        soft_string(
            env,
            cursor,
            jni_str!("getString"),
            jni_sig!("(I)Ljava/lang/String;"),
            &[JValue::Int(column)],
        )
        .filter(|name| !name.is_empty())
    })();
    if let Some(cursor) = cursor.as_ref() {
        // The cursor closes whatever the read did; a leaked one holds the provider open.
        let _ = soft(env, cursor, jni_str!("close"), jni_sig!("()V"), &[]);
    }
    if let Some(from_cursor) = from_cursor {
        return Some(from_cursor);
    }
    let now = soft_static(
        env,
        jni_str!("java/lang/System"),
        jni_str!("currentTimeMillis"),
        jni_sig!("()J"),
        &[],
    )
    .and_then(|now| now.j().ok())?;
    let prefix = match mime {
        Some(mime) if mime.starts_with("image") => "IMG",
        Some(mime) if mime.starts_with("video") => "VID",
        _ => "FILE",
    };
    Some(format!("{prefix}_{}", now))
}

fn java_string_soft<'local>(env: &mut Env<'local>, text: &str) -> Option<JObject<'local>> {
    match env.new_string(text) {
        Ok(value) => Some(value.into()),
        Err(_) => {
            env.exception_clear();
            None
        }
    }
}

/// The file a `content:` URI names, copied into the app's cache the way `receive_sharing_intent`
/// copies it. `None` drops the item: a provider the app cannot read is not a share it can take.
fn copy_content(
    env: &mut Env<'_>,
    context: &JObject<'_>,
    uri: &JObject<'_>,
    mime: Option<&str>,
) -> Option<String> {
    let resolver = soft_object(
        env,
        context,
        jni_str!("getContentResolver"),
        jni_sig!("()Landroid/content/ContentResolver;"),
        &[],
    )?;
    let cache = soft_object(
        env,
        context,
        jni_str!("getCacheDir"),
        jni_sig!("()Ljava/io/File;"),
        &[],
    )?;
    let cache = soft_string(
        env,
        &cache,
        jni_str!("getAbsolutePath"),
        jni_sig!("()Ljava/lang/String;"),
        &[],
    )?;
    let dir = PathBuf::from(cache).join(STAGE_IN);
    std::fs::create_dir_all(&dir).ok()?;
    let name = display_name(env, &resolver, uri, mime)?;
    let target = dir.join(&name);

    let path = java_string_soft(env, &target.to_string_lossy())?;
    let output = match env.new_object(
        jni_str!("java/io/FileOutputStream"),
        jni_sig!("(Ljava/lang/String;)V"),
        &[JValue::Object(&path)],
    ) {
        Ok(output) => output,
        Err(_) => {
            env.exception_clear();
            return None;
        }
    };
    let input = soft_object(
        env,
        &resolver,
        jni_str!("openInputStream"),
        jni_sig!("(Landroid/net/Uri;)Ljava/io/InputStream;"),
        &[JValue::Object(uri)],
    )?;

    // A failed read still closes both ends, so every failure below is caught and cleared
    // before the closes run.
    let mut copied = true;
    let buffer = match env.new_byte_array(64 * 1024) {
        Ok(buffer) => Some(buffer),
        Err(_) => {
            env.exception_clear();
            None
        }
    };
    if let Some(buffer) = buffer {
        let buffer: &JObject<'_> = buffer.as_ref();
        loop {
            let read = soft(
                env,
                &input,
                jni_str!("read"),
                jni_sig!("([B)I"),
                &[JValue::Object(buffer)],
            )
            .and_then(|read| read.i().ok());
            let Some(read) = read else {
                copied = false;
                break;
            };
            if read < 0 {
                break;
            }
            let wrote = soft(
                env,
                &output,
                jni_str!("write"),
                jni_sig!("([BII)V"),
                &[JValue::Object(buffer), JValue::Int(0), JValue::Int(read)],
            );
            if wrote.is_none() {
                copied = false;
                break;
            }
        }
    } else {
        copied = false;
    }
    let _ = soft(env, &output, jni_str!("close"), jni_sig!("()V"), &[]);
    let _ = soft(env, &input, jni_str!("close"), jni_sig!("()V"), &[]);
    if !copied {
        return None;
    }
    Some(target.to_string_lossy().into_owned())
}

/// A video's duration and a PNG frame beside it, through `MediaMetadataRetriever`; a video it
/// cannot read answers neither, never an error, like the reference's guarded read.
fn video_details(env: &mut Env<'_>, path: &str) -> (Option<u64>, Option<String>) {
    let retriever = match env.new_object(
        jni_str!("android/media/MediaMetadataRetriever"),
        jni_sig!("()V"),
        &[],
    ) {
        Ok(retriever) => retriever,
        Err(_) => {
            env.exception_clear();
            return (None, None);
        }
    };
    let details = (|| {
        let source = java_string_soft(env, path)?;
        let _ = soft(
            env,
            &retriever,
            jni_str!("setDataSource"),
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::Object(&source)],
        )?;
        let duration = soft(
            env,
            &retriever,
            jni_str!("extractMetadata"),
            jni_sig!("(I)Ljava/lang/String;"),
            &[JValue::Int(METADATA_DURATION)],
        )
        .and_then(|raw| raw.l().ok())
        .and_then(|raw| read_string_soft(env, raw))
        .and_then(|raw| raw.parse::<u64>().ok());
        let thumbnail = (|| {
            let sdk = match env.get_static_field(
                jni_str!("android/os/Build$VERSION"),
                jni_str!("SDK_INT"),
                jni_sig!("I"),
            ) {
                Ok(sdk) => sdk.i().ok(),
                Err(_) => {
                    env.exception_clear();
                    None
                }
            }?;
            let frame = if sdk >= SDK_SCALED_FRAME {
                soft_object(
                    env,
                    &retriever,
                    jni_str!("getScaledFrameAtTime"),
                    jni_sig!("(JIII)Landroid/graphics/Bitmap;"),
                    &[
                        JValue::Long(-1),
                        JValue::Int(CLOSEST_SYNC),
                        JValue::Int(360),
                        JValue::Int(360),
                    ],
                )
            } else {
                soft_object(
                    env,
                    &retriever,
                    jni_str!("getFrameAtTime"),
                    jni_sig!("(JI)Landroid/graphics/Bitmap;"),
                    &[JValue::Long(-1), JValue::Int(CLOSEST_SYNC)],
                )
            }?;
            let png = match env.get_static_field(
                jni_str!("android/graphics/Bitmap$CompressFormat"),
                jni_str!("PNG"),
                jni_sig!("Landroid/graphics/Bitmap$CompressFormat;"),
            ) {
                Ok(png) => png.l().ok(),
                Err(_) => {
                    env.exception_clear();
                    None
                }
            }?;
            if png.is_null() {
                return None;
            }
            let target = Path::new(path).with_extension("png");
            let out = java_string_soft(env, &target.to_string_lossy())?;
            let output = match env.new_object(
                jni_str!("java/io/FileOutputStream"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&out)],
            ) {
                Ok(output) => output,
                Err(_) => {
                    env.exception_clear();
                    return None;
                }
            };
            let compressed = soft(
                env,
                &frame,
                jni_str!("compress"),
                jni_sig!("(Landroid/graphics/Bitmap$CompressFormat;ILjava/io/OutputStream;)Z"),
                &[
                    JValue::Object(&png),
                    JValue::Int(100),
                    JValue::Object(&output),
                ],
            )
            .and_then(|compressed| compressed.z().ok());
            let _ = soft(env, &output, jni_str!("close"), jni_sig!("()V"), &[]);
            let _ = soft(env, &frame, jni_str!("recycle"), jni_sig!("()V"), &[]);
            compressed?;
            Some(target.to_string_lossy().into_owned())
        })();
        Some((duration, thumbnail))
    })();
    let _ = soft(env, &retriever, jni_str!("release"), jni_sig!("()V"), &[]);
    details.unwrap_or((None, None))
}

/// One shared item from a stream URI: its staged path, its kind, and for a video its duration
/// and a thumbnail. `None` drops the item, as the reference drops one it cannot read.
fn stream_item(
    env: &mut Env<'_>,
    context: &JObject<'_>,
    uri: &JObject<'_>,
    mime: Option<String>,
    message: Option<String>,
) -> Option<SharedMedia> {
    let scheme = soft_string(
        env,
        uri,
        jni_str!("getScheme"),
        jni_sig!("()Ljava/lang/String;"),
        &[],
    );
    let path = match scheme.as_deref() {
        Some("content") => copy_content(env, context, uri, mime.as_deref())?,
        Some("file") => soft_string(
            env,
            uri,
            jni_str!("getPath"),
            jni_sig!("()Ljava/lang/String;"),
            &[],
        )?,
        _ => soft_string(
            env,
            uri,
            jni_str!("toString"),
            jni_sig!("()Ljava/lang/String;"),
            &[],
        )?,
    };
    let mime = match mime {
        Some(mime) => Some(mime),
        None => (|| {
            let name = Path::new(&path).file_name()?.to_str()?;
            let name = java_string_soft(env, name)?;
            let guessed = soft_static(
                env,
                jni_str!("java/net/URLConnection"),
                jni_str!("guessContentTypeFromName"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(&name)],
            )
            .and_then(|guessed| guessed.l().ok())?;
            if guessed.is_null() {
                return None;
            }
            read_string_soft(env, guessed)
        })(),
    };
    let kind = kind_of(mime.as_deref());
    let (duration, thumbnail) = if kind == SharedMediaType::Video {
        video_details(env, &path)
    } else {
        (None, None)
    };
    Some(SharedMedia {
        path,
        thumbnail,
        duration,
        kind,
        mime_type: mime,
        message,
    })
}

fn intent_extra_string(env: &mut Env<'_>, intent: &JObject<'_>, extra: &str) -> Option<String> {
    let key = java_string_soft(env, extra)?;
    soft_string(
        env,
        intent,
        jni_str!("getStringExtra"),
        jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
        &[JValue::Object(&key)],
    )
}

/// The items an intent carries: its stream or its text for `ACTION_SEND`, every stream for
/// `ACTION_SEND_MULTIPLE`. Anything else — `ACTION_MAIN` above all — is no delivery at all.
fn parse_intent(
    env: &mut Env<'_>,
    context: &JObject<'_>,
    intent: &JObject<'_>,
) -> Vec<SharedMedia> {
    let action = soft_string(
        env,
        intent,
        jni_str!("getAction"),
        jni_sig!("()Ljava/lang/String;"),
        &[],
    );
    match action.as_deref() {
        Some(ACTION_SEND) => {
            let mime = soft_string(
                env,
                intent,
                jni_str!("getType"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            );
            let text = intent_extra_string(env, intent, EXTRA_TEXT);
            let Some(key) = java_string_soft(env, EXTRA_STREAM) else {
                return Vec::new();
            };
            let uri = soft_object(
                env,
                intent,
                jni_str!("getParcelableExtra"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&key)],
            );
            if let Some(uri) = uri {
                stream_item(env, context, &uri, mime, text)
                    .into_iter()
                    .collect()
            } else if let Some(text) = text {
                // No file came: the text is the item, the reference's own mapping.
                vec![SharedMedia {
                    path: text.clone(),
                    thumbnail: None,
                    duration: None,
                    kind: SharedMediaType::Text,
                    mime_type: mime,
                    message: Some(text),
                }]
            } else {
                Vec::new()
            }
        }
        Some(ACTION_SEND_MULTIPLE) => {
            let mime = soft_string(
                env,
                intent,
                jni_str!("getType"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            );
            let Some(key) = java_string_soft(env, EXTRA_STREAM) else {
                return Vec::new();
            };
            let streams = soft_object(
                env,
                intent,
                jni_str!("getParcelableArrayListExtra"),
                jni_sig!("(Ljava/lang/String;)Ljava/util/ArrayList;"),
                &[JValue::Object(&key)],
            );
            let count = streams
                .as_ref()
                .and_then(|streams| {
                    soft(env, streams, jni_str!("size"), jni_sig!("()I"), &[])
                        .and_then(|count| count.i().ok())
                })
                .unwrap_or(0);
            // The intent's own MIME is the reduced one of the whole set (`*/*` for a mix); a
            // wildcard names no single item, so each file's extension speaks instead.
            let per_item = mime.filter(|mime| !mime.contains('*'));
            (0..count)
                .filter_map(|at| {
                    let streams = streams.as_ref()?;
                    soft_object(
                        env,
                        streams,
                        jni_str!("get"),
                        jni_sig!("(I)Ljava/lang/Object;"),
                        &[JValue::Int(at)],
                    )
                })
                .filter_map(|uri| stream_item(env, context, &uri, per_item.clone(), None))
                .collect()
        }
        _ => Vec::new(),
    }
}

// --- The intent watcher --------------------------------------------------------------

/// The listener's state: the watcher of the generation that last started one, one waiter, the
/// leftovers of a multi-item delivery (each later `receive` taking one), and the watcher's
/// baseline. A delivery no waiter catches is dropped, as a broadcast stream with no listener
/// drops its event.
struct Listening {
    thread: Option<(u64, JoinHandle<()>)>,
    waiter: Option<MediaDone>,
    queue: VecDeque<SharedMedia>,
    /// What the current watcher started on: the launch intent's identity for the fast path,
    /// and its contents, which suppress a recreated activity re-reading them — and with them
    /// a share that repeats the launch payload exactly, which no signal tells apart. A repeat
    /// of any later delivery is a new share and must arrive.
    baseline: Option<(jint, Vec<SharedMedia>)>,
    /// The identity of the intent the last look saw, whatever it was.
    seen: Option<jint>,
}

/// A stop bumps the generation, retiring the watcher it names; a start may reuse a watcher
/// that still carries the current one.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static LISTENING: OnceLock<Mutex<Listening>> = OnceLock::new();

fn new_listening() -> Mutex<Listening> {
    Mutex::new(Listening {
        thread: None,
        waiter: None,
        queue: VecDeque::new(),
        baseline: None,
        seen: None,
    })
}

fn listening() -> MutexGuard<'static, Listening> {
    LISTENING
        .get_or_init(new_listening)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Keeps exactly one watcher running. A live watcher of the current generation is reused; a
/// retired one is never joined — it exits on its own within one slice of the look it is
/// finishing, however long that look is — and none means a new one starts.
fn ensure_watcher() -> Result<(), ShareError> {
    let mut state = listening();
    if let Some((at, watcher)) = state.thread.take() {
        if at == GENERATION.load(Ordering::SeqCst) && !watcher.is_finished() {
            state.thread = Some((at, watcher));
            return Ok(());
        }
        if watcher.is_finished() {
            let _ = watcher.join();
        }
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    match thread::Builder::new()
        .name("nsh-share-listen".to_owned())
        .spawn(move || watch(generation))
    {
        Ok(watcher) => {
            state.thread = Some((generation, watcher));
            Ok(())
        }
        Err(_) => Err(ShareError::new(ErrorKind::Backend)),
    }
}

/// The identity of the activity's current intent, read in its own frame so the intent's local
/// reference frees at once: the watcher ticks on this, and parses only when it changes.
fn intent_id(env: &mut Env<'_>, context: &JObject<'_>) -> Result<Option<jint>, ShareError> {
    env.with_local_frame(8, |env| {
        let intent = call(
            env,
            context,
            jni_str!("getIntent"),
            jni_sig!("()Landroid/content/Intent;"),
            &[],
        )?
        .l()?;
        if intent.is_null() {
            return Ok(None);
        }
        let id = static_call(
            env,
            jni_str!("java/lang/System"),
            jni_str!("identityHashCode"),
            jni_sig!("(Ljava/lang/Object;)I"),
            &[JValue::Object(&intent)],
        )?
        .i()?;
        Ok(Some(id))
    })
}

/// What the activity's current intent carries, parsed in its own frame so every local
/// reference the parse made frees at once.
fn parse_current(env: &mut Env<'_>, context: &JObject<'_>) -> Result<Vec<SharedMedia>, ShareError> {
    env.with_local_frame(32, |env| {
        let intent = call(
            env,
            context,
            jni_str!("getIntent"),
            jni_sig!("()Landroid/content/Intent;"),
            &[],
        )?
        .l()?;
        if intent.is_null() {
            return Ok(Vec::new());
        }
        Ok(parse_intent(env, context, &intent))
    })
}

/// One look at the activity's intent: a new object with new contents is a delivery, whose
/// first item goes to the waiter and whose rest queue; contents equal to the watcher's
/// baseline (a recreated activity re-reading its launch intent) is not. A watcher's first look
/// only records, so the launch intent counts as the cold start's own and is never delivered
/// twice; a repeat of any later delivery still arrives.
fn watch_once(
    env: &mut Env<'_>,
    context: &JObject<'_>,
    seeded: &mut bool,
) -> Result<(), ShareError> {
    if !is_activity(env, context)? {
        // Without an activity nothing can ever arrive, so a waiting call is answered now
        // instead of being left for a stop that may never come.
        if let Some(waiter) = listening().waiter.take() {
            waiter(Err(ShareError::new(ErrorKind::NotAvailable)));
        }
        return Ok(());
    }
    let Some(id) = intent_id(env, context)? else {
        return Ok(());
    };
    if *seeded && listening().seen == Some(id) {
        return Ok(());
    }
    let media = parse_current(env, context)?;
    let mut state = listening();
    if !*seeded {
        state.baseline = Some((id, media));
        state.seen = Some(id);
        *seeded = true;
        return Ok(());
    }
    state.seen = Some(id);
    // Only a recreated activity re-reading the launch intent is suppressed, by its contents
    // matching the baseline; a repeat of an earlier delivery is a new share and arrives.
    let relaunch = state
        .baseline
        .as_ref()
        .is_some_and(|(_, launch)| *launch == media);
    if relaunch || media.is_empty() {
        return Ok(());
    }
    // A delivery a waiter catches gives it the first item and queues the rest; one no waiter
    // catches is dropped whole, as a broadcast stream drops an unlistened event.
    let Some(waiter) = state.waiter.take() else {
        return Ok(());
    };
    state.queue.extend(media.iter().skip(1).cloned());
    drop(state);
    if let Some(first) = media.first() {
        waiter(Ok(first.clone()));
    }
    Ok(())
}

/// The watcher thread: attached to the VM for its whole life, it looks at the intent twice a
/// second and retires within one 25ms slice of a newer generation.
fn watch(generation: u64) {
    let Some(ptrs) = pointers() else {
        return;
    };
    let attached = run_attached(ptrs, |env, context| {
        let mut seeded = false;
        while generation == GENERATION.load(Ordering::SeqCst) {
            for _ in 0..10 {
                if generation != GENERATION.load(Ordering::SeqCst) {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(25));
            }
            // A failed look is the next look's problem: the watcher leaves the loop only when
            // a newer generation retires it, and never delivers on a failure.
            let _ = watch_once(env, context, &mut seeded);
        }
        Ok(())
    });
    // A watcher that could not attach strands its waiter — it was the only deliverer — so it
    // is answered with a coded failure, and only while this watcher's generation is current:
    // a retired one never resolves a newer session's waiter.
    if attached.is_err() && generation == GENERATION.load(Ordering::SeqCst) {
        if let Some(waiter) = listening().waiter.take() {
            waiter(Err(ShareError::new(ErrorKind::NotAvailable)));
        }
    }
}

// --- The backend --------------------------------------------------------------------

impl Backend for NativeBackend {
    fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(query(false, |env, context| is_activity(env, context)))
    }

    fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(query(false, |env, context| is_activity(env, context)))
    }

    fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError> {
        let plan = plan(params);
        let worker = thread::Builder::new()
            .name("nsh-share-sheet".to_owned())
            .spawn(move || {
                let answer = run_share(plan);
                done(answer);
            });
        // A refused spawn drops `done` unopened: the call fails, and no answer ever comes.
        worker
            .map(|_| ())
            .map_err(|_| ShareError::new(ErrorKind::Backend))
    }

    fn cancel_share(&self) {}

    fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>> {
        Box::pin(query(Vec::new(), |env, context| {
            if !is_activity(env, context)? {
                return Ok(Vec::new());
            }
            parse_current(env, context)
        }))
    }

    fn start_listen(&self, done: MediaDone) -> Result<(), ShareError> {
        if pointers().is_none() {
            // No context at all: nothing could ever deliver, so no waiter is taken.
            return Err(ShareError::new(ErrorKind::NotAvailable));
        }
        if listening().waiter.is_some() {
            return Err(ShareError::new(ErrorKind::InProgress));
        }
        // The watcher starts before anything is answered, so an Err here leaves `done`
        // uncalled, as the contract requires.
        ensure_watcher()?;
        let mut state = listening();
        if let Some(item) = state.queue.pop_front() {
            drop(state);
            done(Ok(item));
            return Ok(());
        }
        if state.waiter.is_some() {
            return Err(ShareError::new(ErrorKind::InProgress));
        }
        state.waiter = Some(done);
        Ok(())
    }

    fn stop_listen(&self) {
        // A newer generation retires the watcher without joining it: it exits on its own
        // within one 25ms slice of whatever look it is finishing, so this thread — the async
        // runtime's, on a dropped receive — is never parked behind a long copy.
        GENERATION.fetch_add(1, Ordering::SeqCst);
        // The watcher was the only deliverer, so a waiter left registered would never be
        // answered; resolving it with a coded failure frees the facade's gate, and a delivery
        // already in flight still arrives through its own hold of the waiter.
        if let Some(waiter) = listening().waiter.take() {
            waiter(Err(ShareError::new(ErrorKind::NotAvailable)));
        }
    }
}
