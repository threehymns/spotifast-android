//! The Android half of the OAuth redirect: catching it while frozen.
//!
//! A fullscreen browser backgrounds this app, and the OS promptly freezes
//! the process, so the loopback listener in [`crate::auth::wait_for_code`]
//! is unreachable when Spotify redirects. The redirect URI is therefore a
//! custom scheme (`rocks.spotifast.spotifast://callback`, registered in
//! manifest.yaml and in Spotify's dashboard), which Android delivers to a
//! *second* NativeActivity instance in this process. That instance never
//! starts the app: [`android_main`](crate::android::android_main) stashes
//! the URI in [`redirect_path`] and finishes it, and the thawed main
//! instance picks the file up on its next frame while a sign-in is pending
//! (see `Command::CheckAuthRedirect`). Split-screen keeps the app visible,
//! so the loopback listener covers that case on its own; both halves race
//! in [`crate::auth::wait_for_code_with_redirect`].

use std::path::PathBuf;
use std::sync::OnceLock;

use jni::Env;
use jni::objects::{Global, JObject, JString, JValue, Reference};
use winit::platform::android::activity::AndroidApp;

use crate::paths::AppDirs;

/// Where a redirect-catcher instance leaves the authorization response URI
/// for the main instance. Removed once consumed, at sign-in start, and at
/// every launch, so a stale file can never complete a later flow.
pub fn redirect_path(dirs: &AppDirs) -> PathBuf {
    dirs.state.join("auth-redirect.txt")
}

/// Atomically stash a caught redirect URI for the main instance.
pub fn stash_redirect(dirs: &AppDirs, uri: &str) {
    let path = redirect_path(dirs);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            log::warn!("unable to create the redirect handoff directory");
            return;
        }
    }
    let temporary = path.with_extension("tmp");
    if std::fs::write(&temporary, uri).is_err()
        || crate::util::replace_file(&temporary, &path).is_err()
    {
        log::warn!("unable to stash the authorization redirect");
    }
}

/// Take and remove a stashed redirect URI, if one is waiting.
pub fn take_redirect(dirs: &AppDirs) -> Option<String> {
    let path = redirect_path(dirs);
    let uri = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    if uri.is_empty() { None } else { Some(uri) }
}

/// Drop any stashed redirect without reading it.
pub fn clear_redirect(dirs: &AppDirs) {
    let _ = std::fs::remove_file(redirect_path(dirs));
}

/// The launch intent's data URI, if any: `activity.getIntent().getData()`
/// as a string. `None` when there is no intent data (a normal launch) or
/// when any JNI call fails; failures are logged, never panicked.
pub fn launch_intent_data(app: &AndroidApp) -> Option<String> {
    // SAFETY: both pointers are valid while `app` is alive, the closure
    // runs attached, and the borrowed activity reference is never freed.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let caught: jni::errors::Result<Option<String>> = vm.attach_current_thread(
        |env| -> jni::errors::Result<Option<String>> {
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        let intent = env
            .call_method(
                &activity,
                jni::jni_str!("getIntent"),
                jni::jni_sig!("()Landroid/content/Intent;"),
                &[],
            )?
            .l()?;
        let data = env
            .call_method(
                &intent,
                jni::jni_str!("getDataString"),
                jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let data: JString = env.cast_local::<JString>(data)?;
        Ok(Some(data.try_to_string(env)?))
    });
    match caught {
        Ok(uri) => uri,
        Err(error) => {
            log::debug!("unable to read the launch intent: {error}");
            None
        }
    }
}

/// Finish a redirect-catcher instance: `activity.finish()`. Best-effort.
pub fn finish_activity(app: &AndroidApp) {
    // SAFETY: as in `launch_intent_data`.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    if let Err(error) = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        env.call_method(
            &activity,
            jni::jni_str!("finish"),
            jni::jni_sig!("()V"),
            &[],
        )?;
        Ok(())
    }) {
        log::debug!("unable to finish the redirect catcher: {error}");
    }
}

/// The main activity, stashed once at startup so backend threads can reach
/// Android APIs (the sign-in keepalive) without an activity handle.
static KEEPALIVE_VM: OnceLock<jni::JavaVM> = OnceLock::new();
static KEEPALIVE_ACTIVITY: OnceLock<Global<JObject<'static>>> = OnceLock::new();

/// Stash the main activity for the sign-in keepalive. Call once from
/// `android_main` after the redirect-catcher early return; further calls
/// are ignored, so a catcher instance can never win the slot.
pub fn init_keepalive(app: &AndroidApp) {
    if KEEPALIVE_VM.get().is_some() {
        return;
    }
    // SAFETY: both pointers are valid while `app` is alive, the closure
    // runs attached, and the global ref outlives the process's need for
    // it (the activity lives as long as the process here).
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let activity = vm.attach_current_thread(|env| {
        let local = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        env.new_global_ref(local)
    });
    match activity {
        Ok(global) => {
            let _ = KEEPALIVE_VM.set(vm);
            let _ = KEEPALIVE_ACTIVITY.set(global);
        }
        Err(error) => log::debug!("unable to stash the main activity: {error}"),
    }
}

/// Start the sign-in keepalive: a foreground service holding the process
/// unfrozen while the browser is in front, so the loopback listener
/// receives Spotify's redirect. Best-effort and idempotent; safe from any
/// thread once [`init_keepalive`] ran.
pub fn start_keepalive() {
    let (Some(vm), Some(activity)) = (KEEPALIVE_VM.get(), KEEPALIVE_ACTIVITY.get()) else {
        return;
    };
    if let Err(error) = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let intent = keepalive_intent(env, activity.as_obj())?;
        env.call_method(
            activity.as_obj(),
            jni::jni_str!("startForegroundService"),
            jni::jni_sig!("(Landroid/content/Intent;)Landroid/content/ComponentName;"),
            &[JValue::from(&intent)],
        )?;
        Ok(())
    }) {
        log::debug!("unable to start the sign-in keepalive: {error}");
    }
}

/// Stop the sign-in keepalive once the flow resolved. Best-effort.
pub fn stop_keepalive() {
    let (Some(vm), Some(activity)) = (KEEPALIVE_VM.get(), KEEPALIVE_ACTIVITY.get()) else {
        return;
    };
    if let Err(error) = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let intent = keepalive_intent(env, activity.as_obj())?;
        env.call_method(
            activity.as_obj(),
            jni::jni_str!("stopService"),
            jni::jni_sig!("(Landroid/content/Intent;)Z"),
            &[JValue::from(&intent)],
        )?;
        Ok(())
    }) {
        log::debug!("unable to stop the sign-in keepalive: {error}");
    }
}

/// `new Intent(activity, AuthKeepaliveService.class)`.
fn keepalive_intent<'env>(
    env: &mut Env<'env>,
    activity: &JObject,
) -> jni::errors::Result<JObject<'env>> {
    let name = jni::jni_str!("rocks/spotifast/spotifast/AuthKeepaliveService");
    let service = env.find_class(name)?;
    // SAFETY: re-wraps the local ref above without taking ownership; the
    // class outlives this call and `JObject` never frees.
    let service = unsafe { JObject::from_raw(env, service.as_raw()) };
    Ok(env.new_object(
        jni::jni_str!("android/content/Intent"),
        jni::jni_sig!("(Landroid/content/Context;Ljava/lang/Class;)V"),
        &[JValue::from(activity), JValue::from(&service)],
    )?)
}
