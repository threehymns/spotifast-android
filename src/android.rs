//! The Android entry point: one activity, one window, no second process.
//!
//! The APK (see manifest.yaml at the repository root) loads this library
//! through NativeActivity, which calls [`android_main`] on its own thread.
//! Startup mirrors the desktop's (src/entrypoint.rs) minus everything a
//! phone has no use for: no command line, no single-instance guard (the
//! system owns the task), no tray to close into, no self-update, and no
//! second window for the Winamp mini player.

use winit::platform::android::activity::AndroidApp;

use crate::app::{App, AppOptions};
use crate::backend::Waker;
use crate::paths::AppDirs;
use crate::settings::Settings;

/// Called by NativeActivity once the activity thread is ready.
// The framework passes its app handle by value; that signature is fixed,
// so the FFI-safety lint is silenced here rather than worked around.
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn android_main(app: AndroidApp) {
    logcat::init();
    // stderr goes nowhere on Android; send panics to logcat instead (with
    // any link in the message removed: a URL can carry a token).
    std::panic::set_hook(Box::new(|info| {
        let message = if let Some(message) = info.payload().downcast_ref::<&str>() {
            message.to_string()
        } else if let Some(message) = info.payload().downcast_ref::<String>() {
            message.clone()
        } else {
            String::from("unknown panic")
        };
        let redacted = fastframe_log::redact::links(message.as_str());
        match info.location() {
            Some(location) => log::error!("panic at {location}: {redacted}"),
            None => log::error!("panic: {redacted}"),
        }
        log::error!("backtrace:\n{:?}", std::backtrace::Backtrace::capture());
    }));

    let dirs = android_dirs(&app);
    let dirs_ready = dirs.ensure();
    if let Err(error) = dirs_ready {
        log::warn!("unable to create the application directories: {error}");
    }
    let settings = Settings::load(&dirs.settings_file());

    let waker = Waker::default();
    let options = AppOptions {
        restore_sign_in: true,
        // The Android media service is a stub (src/media_android.rs), so
        // this exercises the same call sites at no cost.
        media_controls: true,
        tray: false,
    };
    let mut state = App::new(&waker, dirs, settings, options);
    state.enable_desktop_themes();
    state.load_custom_themes(&waker);

    let native_options = eframe::NativeOptions {
        android_app: Some(app),
        viewport: egui::ViewportBuilder::default().with_title("Spotifast"),
        persistence_path: Some(state.dirs.state.join("app.ron")),
        persist_window: true,
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "Spotifast",
        native_options,
        Box::new(move |cc| {
            if let Some(gl) = &cc.gl {
                use eframe::glow::HasContext;
                // eframe has made this window's GL context current before
                // calling the app creator. These identify the renderer
                // actually selected.
                unsafe {
                    log::info!(
                        "OpenGL renderer: {}; vendor: {}; version: {}",
                        gl.get_parameter_string(eframe::glow::RENDERER),
                        gl.get_parameter_string(eframe::glow::VENDOR),
                        gl.get_parameter_string(eframe::glow::VERSION)
                    );
                }
            }
            {
                use raw_window_handle::HasDisplayHandle;
                if let Ok(display) = cc.display_handle() {
                    state.window_level_supported =
                        crate::window::supports_window_level(display.as_raw());
                    state.taskbar_hiding_supported =
                        crate::window::supports_hiding_from_taskbar(display.as_raw());
                }
            }
            state.attach(&cc.egui_ctx);
            Ok(Box::new(Shell { app: state }))
        }),
    ) {
        log::error!("Native window failed: {error}");
    }
}

/// Configuration, state, and caches under the app's internal storage, which
/// needs no permissions. `directories` knows no Android paths, so the
/// activity's own directories are used instead of [`AppDirs::discover`].
fn android_dirs(app: &AndroidApp) -> AppDirs {
    let files = app
        .internal_data_path()
        .or_else(|| app.external_data_path());
    match files {
        Some(files) => {
            let cache = files
                .parent()
                .map(|data| data.join("cache"))
                .unwrap_or_else(|| files.join("cache"));
            AppDirs {
                config: files.join("config"),
                state: files.join("state"),
                cache,
            }
        }
        None => AppDirs::discover(),
    }
}

/// The app's own `log` sink for logcat: one `__android_log_write` call per
/// line, which is all a logging crate would do here, without the dependency.
/// Levels mirror the desktop's default filter (info for this app's crates,
/// warnings and errors everywhere else).
mod logcat {
    use std::ffi::{CString, c_char, c_int};

    const ANDROID_LOG_VERBOSE: c_int = 2;
    const ANDROID_LOG_DEBUG: c_int = 3;
    const ANDROID_LOG_INFO: c_int = 4;
    const ANDROID_LOG_WARN: c_int = 5;
    const ANDROID_LOG_ERROR: c_int = 6;

    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    struct Logger;

    static LOGGER: Logger = Logger;

    impl log::Log for Logger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            let target = metadata.target();
            let max = if target.starts_with("spotifast") || target.starts_with("fastframe_fonts") {
                log::LevelFilter::Info
            } else {
                log::LevelFilter::Warn
            };
            metadata.level() <= max
        }

        fn log(&self, record: &log::Record) {
            if !self.enabled(record.metadata()) {
                return;
            }
            let prio = match record.level() {
                log::Level::Error => ANDROID_LOG_ERROR,
                log::Level::Warn => ANDROID_LOG_WARN,
                log::Level::Info => ANDROID_LOG_INFO,
                log::Level::Debug => ANDROID_LOG_DEBUG,
                log::Level::Trace => ANDROID_LOG_VERBOSE,
            };
            // logcat truncates long lines; the write rejects interior NUL.
            const TAG: &[u8] = b"spotifast\0";
            for line in format!("{}", record.args()).split('\n') {
                if let Ok(text) = CString::new(line.replace('\0', " ")) {
                    unsafe {
                        __android_log_write(prio, TAG.as_ptr() as *const c_char, text.as_ptr());
                    }
                }
            }
        }

        fn flush(&self) {}
    }

    pub fn init() {
        let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Info));
    }
}

/// Owns the app for eframe, like the desktop shell in src/entrypoint.rs.
struct Shell {
    app: App,
}

impl eframe::App for Shell {
    fn persist_egui_memory(&self) -> bool {
        true
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.background_frame(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.frame_ui(ui);
    }

    /// The big window paints itself over eframe's own ground.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.app.settings.winamp_window {
            [0.0; 4]
        } else {
            egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.save_state();
    }
}
