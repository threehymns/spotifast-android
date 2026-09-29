//! Picture-in-picture for the Winamp mini player.
//!
//! The Winamp toggle enters PiP instead of opening a fullscreen skin: the
//! activity shrinks to a floating window while the skin keeps rendering
//! and taking touches (winit redraws while the surface is alive). The
//! manifest flag is in manifest.yaml; the aspect follows the skin stack.
//! All calls are API 26, the minimum supported, so nothing needs a version
//! gate. Failures are logged, never panicked, following `auth_android`.

use jni::objects::{JObject, JValue};

use crate::skin::layout;

/// The system's aspect-ratio bounds for PiP windows: anything outside
/// 1:2.39..2.39:1 throws, so shaded stacks clamp into range.
const MAX_PIP_RATIO: f32 = 2.39;

/// The PiP aspect for a skin stack: the skin width over the stack height,
/// clamped to what the system accepts.
fn pip_aspect(stack_height: u32) -> (i32, i32) {
    let width = layout::WINDOW_WIDTH as f32;
    let height = stack_height.max(1) as f32;
    let ratio = width / height;
    if ratio > MAX_PIP_RATIO {
        (239, 100)
    } else if ratio < 1.0 / MAX_PIP_RATIO {
        (100, 239)
    } else {
        (width as i32, height as i32)
    }
}

/// `new PictureInPictureParams.Builder().setAspectRational(w, h).build()`.
fn pip_params<'env>(
    env: &mut jni::Env<'env>,
    aspect: (i32, i32),
) -> jni::errors::Result<JObject<'env>> {
    let rational = env.new_object(
        jni::jni_str!("android/util/Rational"),
        jni::jni_sig!("(II)V"),
        &[JValue::from(aspect.0), JValue::from(aspect.1)],
    )?;
    let builder = env.new_object(
        jni::jni_str!("android/app/PictureInPictureParams$Builder"),
        jni::jni_sig!("()V"),
        &[],
    )?;
    let builder = env
        .call_method(
            &builder,
            jni::jni_str!("setAspectRational"),
            jni::jni_sig!("(Landroid/util/Rational;)Landroid/app/PictureInPictureParams$Builder;"),
            &[JValue::from(&rational)],
        )?
        .l()?;
    env.call_method(
        &builder,
        jni::jni_str!("build"),
        jni::jni_sig!("()Landroid/app/PictureInPictureParams;"),
        &[],
    )?
    .l()
}

/// Enter PiP with the skin stack's aspect. `false` when PiP is unsupported
/// or disabled, or any JNI call fails: the caller falls back to the
/// fullscreen skin.
///
/// Every outcome is logged at info or above (release builds strip
/// debug): the attempt, a refusal, and any JNI error with its detail.
pub fn enter_winamp_pip(stack_height: u32) -> bool {
    let aspect = pip_aspect(stack_height);
    log::info!(
        "entering picture-in-picture (stack {stack_height}, aspect {}:{})",
        aspect.0,
        aspect.1
    );
    let Some((vm, activity)) = crate::auth_android::activity_and_vm() else {
        log::warn!("picture-in-picture entry without a stashed activity");
        return false;
    };
    let entered: jni::errors::Result<bool> = vm.attach_current_thread(
        |env| -> jni::errors::Result<bool> {
            let params = pip_params(env, aspect)?;
            let entered = env
                .call_method(
                    activity.as_obj(),
                    jni::jni_str!("enterPictureInPictureMode"),
                    jni::jni_sig!("(Landroid/app/PictureInPictureParams;)Z"),
                    &[JValue::from(&params)],
                )?
                .z()?;
            Ok(entered)
        },
    );
    match entered {
        Ok(true) => true,
        Ok(false) => {
            log::warn!("picture-in-picture entry refused by the system");
            false
        }
        Err(error) => {
            log::warn!("unable to enter picture-in-picture: {error}");
            false
        }
    }
}

/// Refresh the PiP aspect after the stack changed height (shade, playlist,
/// equalizer toggles). Outside PiP this only stores the params for the
/// next entry, so callers need no mode check. Best-effort.
pub fn update_winamp_pip(stack_height: u32) {
    let Some((vm, activity)) = crate::auth_android::activity_and_vm() else {
        return;
    };
    if let Err(error) = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let params = pip_params(env, pip_aspect(stack_height))?;
        env.call_method(
            activity.as_obj(),
            jni::jni_str!("setPictureInPictureParams"),
            jni::jni_sig!("(Landroid/app/PictureInPictureParams;)V"),
            &[JValue::from(&params)],
        )?;
        Ok(())
    }) {
        log::warn!("unable to update the picture-in-picture aspect: {error}");
    }
}

/// Leave PiP for the fullscreen window: relaunching the main intent brings
/// our singleTop task to the front, which expands the activity. Already
/// fullscreen, this is a harmless no-op. Best-effort.
pub fn exit_pip_to_fullscreen() {
    let Some((vm, activity)) = crate::auth_android::activity_and_vm() else {
        return;
    };
    if let Err(error) = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let intent = env
            .call_method(
                activity.as_obj(),
                jni::jni_str!("getIntent"),
                jni::jni_sig!("()Landroid/content/Intent;"),
                &[],
            )?
            .l()?;
        env.call_method(
            activity.as_obj(),
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::from(&intent)],
        )?;
        Ok(())
    }) {
        log::warn!("unable to leave picture-in-picture: {error}");
    }
}
