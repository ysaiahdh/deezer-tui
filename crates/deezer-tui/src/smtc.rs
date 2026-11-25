//! System Media Transport Controls (SMTC) integration — Windows only.
//!
//! Exposes the daemon to the Windows media keys, the transport buttons of the
//! volume flyout and the "now playing" panel. This is the Windows counterpart
//! of the MPRIS integration used on Linux ([`crate::mpris`]).
//!
//! Design: the [`SmtcServer`] is a thin adapter. Events coming from the OS are
//! translated into [`Command`]s and forwarded to the daemon over the same
//! media-control channel as MPRIS (so they go through the exact same code path
//! as the TUI client and the `-p`/`-n`/`-b` CLI flags). Playback state is
//! pushed back to the OS by [`SmtcServer::refresh`], which the daemon calls
//! after every command and on every tick.
//!
//! The daemon runs detached, without any window of its own, but SMTC needs an
//! `HWND`. [`SmtcServer`] therefore owns a hidden top-level window that is
//! never shown, and destroys it again on drop.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use tracing::debug;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, RegisterClassW,
    TranslateMessage, UnregisterClassW, HMENU, MSG, PM_REMOVE, WINDOW_EX_STYLE, WNDCLASSW,
    WS_OVERLAPPED,
};

use deezer_core::player::state::{PlaybackStatus, PlayerState};

use crate::protocol::Command;

/// Window class backing the hidden window (unique to this process).
const WINDOW_CLASS: &str = "DeezerTuiSmtcWindow";

/// Step used for `Seek` events, which come from the fast-forward/rewind
/// buttons and do not carry an amount.
const SEEK_STEP_SECS: u64 = 5;

/// A `&str` as a NUL-terminated UTF-16 buffer for the `...W` Win32 APIs.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Build cover-art URL from a Deezer `ALB_PICTURE` md5 hash.
fn cover_url(album_picture: &str) -> Option<String> {
    if album_picture.is_empty() {
        return None;
    }
    Some(format!(
        "https://e-cdns-images.dzcdn.net/images/cover/{album_picture}/264x264-000000-80-0-0.jpg"
    ))
}

/// The metadata fields mirrored onto SMTC, used to detect what changed so the
/// (network-backed, cover art included) metadata update runs only once per
/// track.
#[derive(Clone, Default, PartialEq)]
struct SmtcMetadata {
    track_id: Option<String>,
    title: String,
    artist: String,
    album: String,
    cover_url: Option<String>,
    duration_secs: u64,
}

impl SmtcMetadata {
    fn capture(state: &PlayerState) -> Self {
        let Some(track) = state.current_track.as_ref() else {
            return Self::default();
        };
        Self {
            track_id: Some(track.track_id.clone()),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            cover_url: cover_url(&track.album_picture),
            duration_secs: state.duration_secs,
        }
    }
}

/// Playback fields mirrored onto SMTC (status + timeline position).
#[derive(Clone, Copy, Default, PartialEq)]
struct SmtcPlayback {
    status: PlaybackStatus,
    position_secs: u64,
}

impl SmtcPlayback {
    fn capture(state: &PlayerState) -> Self {
        Self {
            status: state.status,
            position_secs: state.position_secs,
        }
    }

    fn to_media_playback(self) -> MediaPlayback {
        let progress = Some(MediaPosition(Duration::from_secs(self.position_secs)));
        match self.status {
            PlaybackStatus::Playing | PlaybackStatus::Loading => {
                MediaPlayback::Playing { progress }
            }
            PlaybackStatus::Paused => MediaPlayback::Paused { progress },
            PlaybackStatus::Stopped => MediaPlayback::Stopped,
        }
    }
}

/// SMTC adapter: owns the OS controls and the window they are attached to.
pub struct SmtcServer {
    /// `None` only inside [`Drop`], once the controls have been detached.
    controls: Option<MediaControls>,
    hwnd: HWND,
    hinstance: HINSTANCE,
    /// Window class name, kept alive for `UnregisterClassW`.
    class_name: Vec<u16>,
    /// Whether this thread's COM apartment is ours to release.
    com_initialized: bool,
    /// Latest playback status, read from the SMTC event thread so explicit
    /// `Play`/`Pause` requests can be ignored when already in that state.
    status: Arc<Mutex<PlaybackStatus>>,
    metadata: SmtcMetadata,
    playback: SmtcPlayback,
}

impl SmtcServer {
    /// Create the hidden window and register the SMTC controls with it.
    ///
    /// Returns an error if WinRT or SMTC are unavailable (e.g. an old Windows
    /// build); the daemon then simply runs without OS media controls.
    pub fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<Command>) -> Result<Self> {
        // WinRT needs an initialized COM apartment on this thread. MTA lets
        // SMTC deliver its callbacks on a free-threaded RPC thread, so the
        // daemon does not need a UI message loop to receive them.
        let com_initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();

        let (hwnd, hinstance, class_name) = match create_hidden_window() {
            Ok(window) => window,
            Err(e) => {
                if com_initialized {
                    unsafe { CoUninitialize() };
                }
                return Err(e);
            }
        };

        let controls = MediaControls::new(PlatformConfig {
            display_name: "Deezer TUI",
            // Unused on Windows, but part of the cross-platform struct.
            dbus_name: "deezer_tui",
            hwnd: Some(hwnd.0),
        })
        .map_err(|e| anyhow!("SystemMediaTransportControls unavailable: {e:?}"));

        let mut controls = match controls {
            Ok(controls) => controls,
            Err(e) => {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                    let _ = UnregisterClassW(PCWSTR(class_name.as_ptr()), hinstance);
                    if com_initialized {
                        CoUninitialize();
                    }
                }
                return Err(e);
            }
        };

        let status = Arc::new(Mutex::new(PlaybackStatus::Stopped));
        let event_status = Arc::clone(&status);
        if let Err(e) = controls.attach(move |event| {
            handle_event(&event_status, &cmd_tx, event);
        }) {
            unsafe {
                let _ = DestroyWindow(hwnd);
                let _ = UnregisterClassW(PCWSTR(class_name.as_ptr()), hinstance);
                if com_initialized {
                    CoUninitialize();
                }
            }
            return Err(anyhow!("SMTC event handler registration failed: {e:?}"));
        }

        Ok(Self {
            controls: Some(controls),
            hwnd,
            hinstance,
            class_name,
            com_initialized,
            status,
            metadata: SmtcMetadata::default(),
            playback: SmtcPlayback::default(),
        })
    }

    /// Push the player state to SMTC, skipping updates that changed nothing.
    pub fn refresh(&mut self, state: &PlayerState) {
        if let Ok(mut status) = self.status.lock() {
            *status = state.status;
        }

        // The daemon has no message loop: drain anything Windows queued for
        // the window so it does not pile up unprocessed.
        pump_messages();

        let Some(controls) = self.controls.as_mut() else {
            return;
        };

        let metadata = SmtcMetadata::capture(state);
        if metadata != self.metadata {
            // A stop clears the track: keep the last item on screen, the
            // `Stopped` playback status already tells the OS nothing plays.
            if metadata.track_id.is_some() {
                let update = MediaMetadata {
                    title: Some(metadata.title.as_str()),
                    artist: Some(metadata.artist.as_str()),
                    album: (!metadata.album.is_empty()).then_some(metadata.album.as_str()),
                    cover_url: metadata.cover_url.as_deref(),
                    duration: (metadata.duration_secs > 0)
                        .then(|| Duration::from_secs(metadata.duration_secs)),
                };
                if let Err(e) = controls.set_metadata(update) {
                    debug!("SMTC set_metadata failed: {e:?}");
                }
            }
            self.metadata = metadata;
        }

        let playback = SmtcPlayback::capture(state);
        if playback != self.playback {
            if let Err(e) = controls.set_playback(playback.to_media_playback()) {
                debug!("SMTC set_playback failed: {e:?}");
            }
            self.playback = playback;
        }
    }
}

impl Drop for SmtcServer {
    fn drop(&mut self) {
        // Detach the controls first: they must still have their window.
        drop(self.controls.take());
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(PCWSTR(self.class_name.as_ptr()), self.hinstance);
            if self.com_initialized {
                CoUninitialize();
            }
        }
    }
}

/// Register the window class and create the hidden top-level window SMTC
/// needs.
///
/// A message-only window (`HWND_MESSAGE`) is *not* usable here:
/// `ISystemMediaTransportControlsInterop::GetForWindow` rejects it with
/// `E_INVALIDARG`. A normal top-level window that is never shown is accepted
/// and still invisible to the user (no taskbar entry, no Alt+Tab entry).
fn create_hidden_window() -> Result<(HWND, HINSTANCE, Vec<u16>)> {
    let module =
        unsafe { GetModuleHandleW(None) }.map_err(|e| anyhow!("GetModuleHandleW failed: {e}"))?;
    let hinstance = HINSTANCE(module.0);

    let class_name = wide(WINDOW_CLASS);
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: hinstance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(anyhow!("RegisterClassW failed for {WINDOW_CLASS}"));
    }

    let title = wide("Deezer TUI");
    // No `WS_VISIBLE`: the window only exists to give SMTC an `HWND`.
    let created = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )
    };

    match created {
        Ok(hwnd) => Ok((hwnd, hinstance, class_name)),
        Err(e) => {
            unsafe { UnregisterClassW(PCWSTR(class_name.as_ptr()), hinstance) }.ok();
            Err(anyhow!("CreateWindowExW failed: {e}"))
        }
    }
}

/// Default window procedure: the window only exists to host SMTC.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Non-blocking drain of the calling thread's message queue.
fn pump_messages() {
    // Bounded so a flood of messages can never stall the daemon loop.
    const MAX_MESSAGES: usize = 32;

    let mut msg = MSG::default();
    for _ in 0..MAX_MESSAGES {
        let has_message = unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE) };
        if !has_message.as_bool() {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Translate one SMTC event into a daemon command.
fn handle_event(
    status: &Mutex<PlaybackStatus>,
    cmd_tx: &tokio::sync::mpsc::UnboundedSender<Command>,
    event: MediaControlEvent,
) {
    let current = status.lock().map(|s| *s).unwrap_or_default();
    let command = match event {
        // SMTC has both explicit and toggle buttons: only act when the request
        // changes something, so a stale "play" cannot pause.
        MediaControlEvent::Play => {
            (current == PlaybackStatus::Paused).then_some(Command::TogglePause)
        }
        MediaControlEvent::Pause => {
            (current == PlaybackStatus::Playing).then_some(Command::TogglePause)
        }
        MediaControlEvent::Toggle => Some(Command::TogglePause),
        MediaControlEvent::Next => Some(Command::NextTrack),
        MediaControlEvent::Previous => Some(Command::PrevTrack),
        MediaControlEvent::Stop => Some(Command::Stop),
        MediaControlEvent::Seek(direction) => Some(seek_command(direction, SEEK_STEP_SECS)),
        MediaControlEvent::SeekBy(direction, amount) => {
            Some(seek_command(direction, amount.as_secs().max(1)))
        }
        MediaControlEvent::SetPosition(MediaPosition(position)) => Some(Command::SeekAbsolute {
            secs: position.as_secs(),
        }),
        MediaControlEvent::SetVolume(volume) => Some(Command::SetVolume {
            volume: volume.clamp(0.0, 1.0) as f32,
        }),
        // The daemon has no window to bring to front and does not open URIs.
        MediaControlEvent::Raise | MediaControlEvent::OpenUri(_) => None,
        MediaControlEvent::Quit => Some(Command::Shutdown),
    };

    if let Some(command) = command {
        let _ = cmd_tx.send(command);
    }
}

fn seek_command(direction: SeekDirection, secs: u64) -> Command {
    match direction {
        SeekDirection::Forward => Command::SeekForward { secs },
        SeekDirection::Backward => Command::SeekBackward { secs },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_url_built_from_album_picture() {
        assert_eq!(
            cover_url("0123456789abcdef").as_deref(),
            Some(
                "https://e-cdns-images.dzcdn.net/images/cover/0123456789abcdef/264x264-000000-80-0-0.jpg"
            )
        );
        assert_eq!(cover_url(""), None);
    }

    #[test]
    fn playback_mapping_follows_status() {
        let playing = SmtcPlayback {
            status: PlaybackStatus::Playing,
            position_secs: 12,
        };
        assert_eq!(
            playing.to_media_playback(),
            MediaPlayback::Playing {
                progress: Some(MediaPosition(Duration::from_secs(12)))
            }
        );

        let loading = SmtcPlayback {
            status: PlaybackStatus::Loading,
            position_secs: 0,
        };
        assert!(matches!(
            loading.to_media_playback(),
            MediaPlayback::Playing { .. }
        ));

        let paused = SmtcPlayback {
            status: PlaybackStatus::Paused,
            position_secs: 12,
        };
        assert!(matches!(
            paused.to_media_playback(),
            MediaPlayback::Paused { .. }
        ));

        let stopped = SmtcPlayback {
            status: PlaybackStatus::Stopped,
            position_secs: 0,
        };
        assert_eq!(stopped.to_media_playback(), MediaPlayback::Stopped);
    }

    #[test]
    fn seek_events_map_to_relative_commands() {
        assert!(matches!(
            seek_command(SeekDirection::Forward, 5),
            Command::SeekForward { secs: 5 }
        ));
        assert!(matches!(
            seek_command(SeekDirection::Backward, 5),
            Command::SeekBackward { secs: 5 }
        ));
    }
}
