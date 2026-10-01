//! Android media controls placeholder for Spotifast.
//!
//! Playback itself works; the lock-screen and notification controls are not
//! wired yet (that needs a MediaSession bridge over JNI). The stub keeps the
//! interface's media-control call sites identical across platforms: commands
//! never arrive and state updates are dropped.

use crate::media::{MediaCommand, MediaState};

pub struct MediaService {
    commands: std::sync::mpsc::Receiver<MediaCommand>,
}

impl MediaService {
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let _ = wake;
        let (_, commands) = std::sync::mpsc::channel();
        Self { commands }
    }

    pub fn drain_commands(&self) -> Vec<MediaCommand> {
        self.commands.try_iter().collect()
    }

    pub fn update(&mut self, state: MediaState) {
        let _ = state;
    }

    pub fn seeked(&self, position_ms: u32) {
        let _ = position_ms;
    }
}
