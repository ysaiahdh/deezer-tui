use std::io;
use std::path::PathBuf;
#[cfg(windows)]
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use deezer_core::api::models::{
    AlbumDetail, ArtistDetail, ArtistSubTab, AudioQuality, DisplayItem, GenreDetail, PlaylistData,
    PlaylistDetail, TrackData,
};
use deezer_core::offline::OfflineTrack;
use deezer_core::player::eq::EqSettings;
use deezer_core::player::state::{PlaybackStatus, RepeatMode};

/// Commands sent from the TUI client to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Request a full state snapshot.
    GetSnapshot,
    /// Login with ARL token.
    Login { arl: String },
    /// Search for tracks.
    Search { query: String },
    /// Play track at index from search results.
    PlayFromSearch { index: usize },
    /// Play track at index from favorites.
    PlayFromFavorites { index: usize },
    /// Toggle play/pause.
    TogglePause,
    /// Next track in queue.
    NextTrack,
    /// Previous track in queue.
    PrevTrack,
    /// Set volume (0.0 - 1.0).
    SetVolume { volume: f32 },
    /// Seek forward by a number of seconds in the current track.
    SeekForward { secs: u64 },
    /// Seek backward by a number of seconds in the current track.
    SeekBackward { secs: u64 },
    /// Seek to an absolute position (seconds) in the current track.
    SeekAbsolute { secs: u64 },
    /// Stop playback entirely (clears current track, resets position).
    Stop,
    /// Toggle shuffle mode.
    ToggleShuffle,
    /// Cycle repeat mode (Off -> Queue -> Track -> Off).
    CycleRepeat,
    /// Force-refresh the current favorites category from Deezer.
    RefreshFavorites,
    /// Navigate list selection up.
    SelectUp,
    /// Navigate list selection down.
    SelectDown,
    /// Switch to next tab.
    NextTab,
    /// Switch to previous tab.
    PrevTab,
    /// Switch to next category within current tab.
    NextCategory,
    /// Switch to previous category within current tab.
    PrevCategory,
    /// Switch directly to a tab (mouse click on the tab bar).
    SetTab { tab: ActiveTab },
    /// Switch directly to a category of the current tab, by index into its
    /// `ALL` list (mouse click on the category menu).
    SetCategory { index: usize },
    /// Set the current tab's list selection directly (mouse click on a row).
    SelectIndex { index: usize },
    /// Play favorites in shuffle mode.
    ShuffleFavorites,
    /// Add a track to favorites.
    AddFavorite { track_id: String },
    /// Remove a track from favorites.
    RemoveFavorite { track_id: String },
    /// Add an artist to favorites.
    AddFavoriteArtist { artist_id: String },
    /// Remove an artist from favorites.
    RemoveFavoriteArtist { artist_id: String },
    /// Add an album to favorites.
    AddFavoriteAlbum { album_id: String },
    /// Remove an album from favorites.
    RemoveFavoriteAlbum { album_id: String },
    /// Request the user's playlists (for playlist picker).
    RequestPlaylists,
    /// Add a track to a playlist.
    AddToPlaylist {
        playlist_id: String,
        track_id: String,
    },
    /// Remove a track from a playlist (requires user to own the playlist).
    RemoveFromPlaylist {
        playlist_id: String,
        track_id: String,
    },
    /// Create a new playlist and add a track to it.
    CreatePlaylistAndAdd { title: String, track_id: String },
    /// Rename an existing playlist.
    RenamePlaylist {
        playlist_id: String,
        new_title: String,
    },
    /// Delete a playlist (owner only).
    DeletePlaylist { playlist_id: String },
    /// Mark a track as disliked (don't recommend).
    DislikeTrack { track_id: String },
    /// Insert a track to play next in the queue.
    PlayNext { track: TrackData },
    /// Append a track to the end of the queue.
    AddToQueue { track: TrackData },
    /// Start a mix inspired by a track.
    StartMix { track_id: String },
    /// Start Deezer Flow (personalized radio).
    StartFlow,
    /// Jump to and play a specific track in the queue by index.
    PlayFromQueue { index: usize },
    /// Remove a track from the queue by index.
    RemoveFromQueue { index: usize },
    /// Load album detail (tracks, metadata).
    GetAlbumDetail { album_id: String },
    /// Play a track from the album detail view.
    PlayFromAlbum { index: usize },
    /// Load artist detail (top tracks, albums).
    GetArtistDetail { artist_id: String },
    /// Play a track from the artist detail top tracks.
    PlayFromArtist { index: usize },
    /// Open an album from the artist detail albums list.
    OpenArtistAlbum { index: usize },
    /// Load playlist detail (tracks, metadata).
    GetPlaylistDetail { playlist_id: String },
    /// Play a track from the playlist detail view. `order` is the playlist's
    /// display order (track indices) when the user sorted it, so the queue
    /// follows what is on screen; empty means the playlist's own order.
    PlayFromPlaylist {
        index: usize,
        #[serde(default)]
        order: Vec<usize>,
    },
    /// Load a podcast show's episode list.
    GetShowDetail { show_id: String },
    /// Logout — clear ARL, return to login screen.
    Logout,
    /// Load radio stations from Deezer.
    LoadRadios,
    /// Play the selected radio station's tracks.
    PlayFromRadio { index: usize },
    /// Load the mood channels list from Deezer.
    LoadMoods,
    /// Play the selected mood (resolves tracks via the API).
    PlayFromMood { index: usize },
    /// Load music genres/categories from Deezer.
    LoadGenres,
    /// Load the detail (chart + radios) of a specific genre.
    GetGenreDetail { genre_id: u64, name: String },
    /// Play a track from the genre detail Tracks sub-tab.
    PlayFromGenreTrack { index: usize },
    /// Play a radio from the genre detail Radios sub-tab.
    PlayFromGenreRadio { index: usize },
    /// Download a track for offline mode.
    DownloadOffline { track: TrackData },
    /// Download an entire album for offline mode.
    DownloadAlbumOffline { album_id: String },
    /// Download an entire playlist for offline mode.
    DownloadPlaylistOffline { playlist_id: String },
    /// Remove a track from offline storage.
    RemoveOfflineTrack { track_id: String },
    /// Remove an album from offline storage.
    RemoveOfflineAlbum { album_id: String },
    /// Remove a playlist from offline storage.
    RemoveOfflinePlaylist { playlist_id: String },
    /// Play a track from offline storage.
    PlayFromOffline { index: usize },
    /// Play a track from an offline album (queue = album tracks).
    PlayOfflineAlbum {
        album_id: String,
        track_index: usize,
    },
    /// Play a track from an offline playlist (queue = playlist tracks).
    PlayOfflinePlaylist {
        playlist_id: String,
        track_index: usize,
    },
    /// Push a navigation overlay onto the daemon-side stack.
    PushNavOverlay(NavOverlay),
    /// Pop the top navigation overlay from the daemon-side stack.
    PopNavOverlay,
    /// Clear the entire navigation overlay stack (e.g. when entering a new top-level detail).
    ClearNavOverlayStack,
    /// Change the preferred audio quality (persisted to config). Takes effect on next track.
    SetQuality { quality: AudioQuality },
    /// Replace the equalizer settings (applied live, persisted to config).
    SetEqualizer { settings: EqSettings },
    /// Graceful shutdown — daemon exits.
    Shutdown,
}

/// Navigation overlays that are persisted in the daemon across client reconnections.
/// Only content-detail views that correspond to loaded daemon data belong here.
/// Pure UI overlays (Help, Settings, WaitingList, etc.) stay client-side only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavOverlay {
    ArtistDetail,
    AlbumDetail { from_artist: bool },
    PlaylistDetail,
    GenreDetail,
}

/// Sub-categories within the GenreDetail overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GenreDetailSubTab {
    #[default]
    Tracks,
    Albums,
    Artists,
    Playlists,
    Radios,
}

impl GenreDetailSubTab {
    pub const ALL: [Self; 5] = [
        Self::Tracks,
        Self::Albums,
        Self::Artists,
        Self::Playlists,
        Self::Radios,
    ];

    pub fn next(&self) -> Self {
        let all = Self::ALL;
        let idx = all.iter().position(|c| c == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    pub fn prev(&self) -> Self {
        let all = Self::ALL;
        let idx = all.iter().position(|c| c == self).unwrap_or(0);
        all[(idx + all.len() - 1) % all.len()]
    }
}

/// Messages sent from the daemon to the TUI client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Full state snapshot.
    Snapshot(DaemonSnapshot),
    /// Error message.
    Error(String),
}

/// Screen the daemon is on (login vs main).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Screen {
    #[default]
    Login,
    Main,
}

/// Active tab in main view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ActiveTab {
    #[default]
    Search,
    Favorites,
    Explore,
    Downloads,
}

/// Sub-categories within the Explore tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExploreCategory {
    #[default]
    Moods,
    Categories,
    Radios,
}

impl ExploreCategory {
    pub const ALL: [Self; 3] = [Self::Moods, Self::Categories, Self::Radios];
}

/// A music genre/category for display.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenreItem {
    pub id: u64,
    pub name: String,
}

/// Search category filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SearchCategory {
    /// Mixed results of every type (top result first).
    #[default]
    All,
    Track,
    Artist,
    Album,
    Playlist,
    Podcast,
    Episode,
    Profile,
}

impl SearchCategory {
    pub const ALL: [Self; 8] = [
        Self::All,
        Self::Track,
        Self::Artist,
        Self::Album,
        Self::Playlist,
        Self::Podcast,
        Self::Episode,
        Self::Profile,
    ];

    /// API section key used in deezer.pageSearch response.
    pub fn api_key(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Track => "TRACK",
            Self::Artist => "ARTIST",
            Self::Album => "ALBUM",
            Self::Playlist => "PLAYLIST",
            Self::Podcast => "SHOW",
            Self::Episode => "EPISODE",
            Self::Profile => "USER",
        }
    }
}

/// Favorites category filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FavoritesCategory {
    #[default]
    RecentlyPlayed,
    Tracks,
    Artists,
    Albums,
    Playlists,
    Following,
}

impl FavoritesCategory {
    pub const ALL: [Self; 6] = [
        Self::RecentlyPlayed,
        Self::Tracks,
        Self::Artists,
        Self::Albums,
        Self::Playlists,
        Self::Following,
    ];
}

/// Offline content category filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OfflineCategory {
    #[default]
    Tracks,
    Albums,
    Playlists,
}

impl OfflineCategory {
    pub const ALL: [Self; 3] = [Self::Tracks, Self::Albums, Self::Playlists];
}

/// A radio station item for display.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RadioItem {
    pub id: u64,
    pub title: String,
}

/// A mood channel item for display in the Explore > Moods tab.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MoodEntry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub radio_id: Option<u64>,
}

/// Complete state snapshot sent from daemon to client.
/// All fields use `#[serde(default)]` for forward/backward compatibility
/// when daemon and client are running different binary versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSnapshot {
    #[serde(default)]
    pub screen: Screen,
    #[serde(default)]
    pub active_tab: ActiveTab,

    // Player state
    #[serde(default)]
    pub status: PlaybackStatus,
    #[serde(default)]
    pub current_track: Option<TrackData>,
    #[serde(default)]
    pub quality: AudioQuality,
    #[serde(default)]
    pub position_secs: u64,
    #[serde(default)]
    pub duration_secs: u64,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default)]
    pub equalizer: EqSettings,

    // Queue
    #[serde(default)]
    pub queue: Vec<TrackData>,
    #[serde(default)]
    pub queue_index: usize,

    // Search
    #[serde(default)]
    pub search_results: Vec<TrackData>,
    #[serde(default)]
    pub search_selected: usize,
    #[serde(default)]
    pub search_loading: bool,
    #[serde(default)]
    pub search_category: SearchCategory,
    #[serde(default)]
    pub search_display: Vec<DisplayItem>,

    // Favorites
    #[serde(default)]
    pub favorites: Vec<TrackData>,
    #[serde(default)]
    pub favorites_selected: usize,
    #[serde(default)]
    pub favorites_loading: bool,
    #[serde(default)]
    pub favorites_category: FavoritesCategory,
    #[serde(default)]
    pub favorites_display: Vec<DisplayItem>,
    /// IDs of tracks in the user's favorites (Loved Tracks).
    #[serde(default)]
    pub favorite_track_ids: Vec<String>,
    /// IDs of artists in the user's favorites.
    #[serde(default)]
    pub favorite_artist_ids: Vec<String>,
    /// IDs of albums in the user's favorites.
    #[serde(default)]
    pub favorite_album_ids: Vec<String>,

    // Offline
    #[serde(default)]
    pub offline_category: OfflineCategory,
    #[serde(default)]
    pub offline_tracks: Vec<OfflineTrack>,
    #[serde(default)]
    pub offline_albums: Vec<AlbumDetail>,
    #[serde(default)]
    pub offline_playlists: Vec<PlaylistDetail>,
    #[serde(default)]
    pub offline_selected: usize,
    #[serde(default)]
    pub offline_loading: bool,
    /// IDs of tracks available offline (for UI indicators in other tabs).
    #[serde(default)]
    pub offline_track_ids: Vec<String>,

    // Explore tab
    #[serde(default)]
    pub explore_category: ExploreCategory,

    // Radios
    #[serde(default)]
    pub radios: Vec<RadioItem>,
    #[serde(default)]
    pub radios_selected: usize,
    #[serde(default)]
    pub radios_loading: bool,

    // Moods
    #[serde(default)]
    pub moods: Vec<MoodEntry>,
    #[serde(default)]
    pub moods_selected: usize,
    #[serde(default)]
    pub moods_loading: bool,

    // Genres / Categories
    #[serde(default)]
    pub genres: Vec<GenreItem>,
    #[serde(default)]
    pub genres_selected: usize,
    #[serde(default)]
    pub genres_loading: bool,

    // Playlists (for playlist picker in popup menu)
    #[serde(default)]
    pub playlists: Vec<PlaylistData>,

    // Album detail
    #[serde(default)]
    pub album_detail: Option<AlbumDetail>,
    #[serde(default)]
    pub album_detail_selected: usize,
    #[serde(default)]
    pub album_detail_loading: bool,

    // Artist detail
    #[serde(default)]
    pub artist_detail: Option<ArtistDetail>,
    #[serde(default)]
    pub artist_detail_selected: usize,
    #[serde(default)]
    pub artist_detail_loading: bool,
    #[serde(default)]
    pub artist_detail_sub_tab: ArtistSubTab,

    // Playlist detail
    #[serde(default)]
    pub playlist_detail: Option<PlaylistDetail>,
    #[serde(default)]
    pub playlist_detail_selected: usize,
    #[serde(default)]
    pub playlist_detail_loading: bool,

    // Genre detail
    #[serde(default)]
    pub genre_detail: Option<GenreDetail>,
    #[serde(default)]
    pub genre_detail_loading: bool,

    // Navigation overlay stack (persisted across reconnections)
    #[serde(default)]
    pub nav_overlay: Option<NavOverlay>,
    #[serde(default)]
    pub nav_overlay_stack: Vec<NavOverlay>,

    // UI hints
    #[serde(default)]
    pub status_msg: Option<String>,
    #[serde(default)]
    pub login_error: Option<String>,
    #[serde(default)]
    pub login_loading: bool,
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default)]
    pub is_offline: bool,
}

fn default_volume() -> f32 {
    0.8
}

impl Default for DaemonSnapshot {
    fn default() -> Self {
        Self {
            screen: Screen::Login,
            active_tab: ActiveTab::Search,
            status: PlaybackStatus::Stopped,
            current_track: None,
            quality: AudioQuality::Mp3_128,
            position_secs: 0,
            duration_secs: 0,
            volume: 0.8,
            shuffle: false,
            repeat: RepeatMode::Off,
            equalizer: EqSettings::default(),
            queue: Vec::new(),
            queue_index: 0,
            search_results: Vec::new(),
            search_selected: 0,
            search_loading: false,
            search_category: SearchCategory::default(),
            search_display: Vec::new(),
            favorites: Vec::new(),
            favorites_selected: 0,
            favorites_loading: false,
            favorites_category: FavoritesCategory::default(),
            favorites_display: Vec::new(),
            favorite_track_ids: Vec::new(),
            favorite_artist_ids: Vec::new(),
            favorite_album_ids: Vec::new(),
            offline_category: OfflineCategory::default(),
            offline_tracks: Vec::new(),
            offline_albums: Vec::new(),
            offline_playlists: Vec::new(),
            offline_selected: 0,
            offline_loading: false,
            offline_track_ids: Vec::new(),
            explore_category: ExploreCategory::default(),
            radios: Vec::new(),
            radios_selected: 0,
            radios_loading: false,
            moods: Vec::new(),
            moods_selected: 0,
            moods_loading: false,
            genres: Vec::new(),
            genres_selected: 0,
            genres_loading: false,
            playlists: Vec::new(),
            album_detail: None,
            album_detail_selected: 0,
            album_detail_loading: false,
            artist_detail: None,
            artist_detail_selected: 0,
            artist_detail_loading: false,
            artist_detail_sub_tab: ArtistSubTab::default(),
            playlist_detail: None,
            playlist_detail_selected: 0,
            playlist_detail_loading: false,
            genre_detail: None,
            genre_detail_loading: false,
            nav_overlay: None,
            nav_overlay_stack: Vec::new(),
            status_msg: None,
            login_error: None,
            login_loading: false,
            user_name: None,
            is_offline: false,
        }
    }
}

/// Address of the daemon's IPC endpoint.
///
/// Unix uses a socket file in the config directory. Windows has no Unix-domain
/// sockets in Tokio, so the daemon listens on a named pipe scoped to the
/// current user.
pub fn socket_path() -> PathBuf {
    #[cfg(unix)]
    {
        deezer_core::Config::dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("daemon.sock")
    }
    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\deezer-tui-{}", pipe_user()))
    }
}

/// Path of the daemon PID file (used to stop a stuck daemon on update).
pub fn pid_path() -> PathBuf {
    deezer_core::Config::dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("daemon.pid")
}

/// Windows user name, restricted to characters that are safe in a pipe name.
#[cfg(windows)]
fn pipe_user() -> String {
    let raw = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "default".to_string());
    let safe: String = raw
        .chars()
        .take(128)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "default".to_string()
    } else {
        safe
    }
}

/// Client end of the daemon connection.
#[cfg(unix)]
pub type ClientStream = tokio::net::UnixStream;
#[cfg(windows)]
pub type ClientStream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Server end of a daemon connection.
#[cfg(unix)]
pub type ServerStream = tokio::net::UnixStream;
#[cfg(windows)]
pub type ServerStream = tokio::net::windows::named_pipe::NamedPipeServer;

pub type ClientWriteHalf = tokio::io::WriteHalf<ClientStream>;
pub type ServerWriteHalf = tokio::io::WriteHalf<ServerStream>;

/// Windows error returned while every pipe instance is serving a client.
#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;

/// Connect to the daemon.
pub async fn connect() -> io::Result<ClientStream> {
    connect_at(socket_path()).await
}

async fn connect_at(path: PathBuf) -> io::Result<ClientStream> {
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(path).await
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;

        // `ERROR_PIPE_BUSY` means the daemon is alive but every instance is
        // taken: wait for one to come free instead of reporting it as down.
        for _ in 0..40 {
            match ClientOptions::new().open(&path) {
                Ok(client) => return Ok(client),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "daemon pipe stayed busy",
        ))
    }
}

/// Synchronous liveness probe used before starting the daemon.
pub fn connect_sync() -> bool {
    connect_sync_at(socket_path())
}

fn connect_sync_at(path: PathBuf) -> bool {
    #[cfg(unix)]
    {
        if !path.exists() {
            return false;
        }
        match std::os::unix::net::UnixStream::connect(&path) {
            Ok(_stream) => true,
            Err(_) => {
                // Stale socket file — clean it up so a fresh daemon can bind.
                let _ = std::fs::remove_file(&path);
                false
            }
        }
    }
    #[cfg(windows)]
    {
        for _ in 0..20 {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
            {
                Ok(_stream) => return true,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => return false,
            }
        }
        false
    }
}

/// Listening end of the daemon IPC channel.
pub struct IpcListener {
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    pending: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
}

impl IpcListener {
    /// Bind the IPC endpoint, replacing a stale Unix socket file if needed.
    pub async fn bind() -> io::Result<Self> {
        Self::bind_at(socket_path()).await
    }

    async fn bind_at(path: PathBuf) -> io::Result<Self> {
        #[cfg(unix)]
        {
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Ok(Self {
                inner: tokio::net::UnixListener::bind(&path)?,
            })
        }
        #[cfg(windows)]
        {
            let pending = new_pipe_instance(&path, true)?;
            Ok(Self {
                path,
                pending: Some(pending),
            })
        }
    }

    /// Wait for the next client and hand back the connected stream.
    pub async fn accept(&mut self) -> io::Result<ServerStream> {
        #[cfg(unix)]
        {
            let (stream, _addr) = self.inner.accept().await?;
            Ok(stream)
        }
        #[cfg(windows)]
        {
            if self.pending.is_none() {
                self.pending = Some(new_pipe_instance(&self.path, false)?);
            }

            // Await on the instance already stored in `self`: this future is
            // polled inside `select!` and may be cancelled at any await point,
            // so it must not take the instance out before the handshake ends.
            let connected = self
                .pending
                .as_ref()
                .expect("instance created above")
                .connect()
                .await;

            if let Err(e) = connected {
                // The client vanished mid-handshake, so this instance is dead.
                // Recreate the pending slot right away to keep the pipe name
                // available; if recreation fails, leave it empty so the next
                // `accept` retries instead of reusing a dead handle.
                self.pending = new_pipe_instance(&self.path, false).ok();
                return Err(e);
            }

            let connected = self.pending.take().expect("instance existed");
            // Create the next instance before returning, so another client can
            // connect while this one is being served.
            self.pending = Some(new_pipe_instance(&self.path, false)?);
            Ok(connected)
        }
    }
}

/// Create one instance of the daemon's named pipe.
#[cfg(windows)]
fn new_pipe_instance(
    path: &std::path::Path,
    first: bool,
) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut options = ServerOptions::new();
    // The first instance claims the name: if it already exists, another daemon
    // is running and binding must fail.
    options.first_pipe_instance(first);
    options.create(path)
}

/// Send a line-delimited JSON message over any async stream.
pub async fn send_line<W, T>(stream: &mut W, msg: &T) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut json = serde_json::to_string(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    json.push('\n');
    stream.write_all(json.as_bytes()).await?;
    stream.flush().await
}

/// Read a line-delimited JSON message from a buffered reader.
pub async fn read_line<T: for<'de> Deserialize<'de>, R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> std::io::Result<Option<T>> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None); // EOF — peer disconnected
    }
    let msg = serde_json::from_str(&line)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

#[cfg(test)]
mod ipc_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Unique endpoint per test run, so a real daemon (or a parallel test) is
    /// never disturbed.
    fn test_endpoint() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        #[cfg(unix)]
        {
            std::env::temp_dir().join(format!("deezer-tui-test-{pid}-{n}.sock"))
        }
        #[cfg(windows)]
        {
            PathBuf::from(format!(r"\\.\pipe\deezer-tui-test-{pid}-{n}"))
        }
    }

    #[tokio::test]
    async fn ipc_roundtrip_and_second_client() {
        let path = test_endpoint();
        let mut listener = IpcListener::bind_at(path.clone()).await.unwrap();

        let first_path = path.clone();
        let first = tokio::spawn(async move {
            let mut stream = connect_at(first_path).await.unwrap();
            send_line(&mut stream, &"ping".to_string()).await.unwrap();
        });

        let stream = listener.accept().await.unwrap();
        let (read_half, _write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let got: Option<String> = read_line(&mut reader).await.unwrap();
        assert_eq!(got.as_deref(), Some("ping"));
        first.await.unwrap();

        // A second client must also get through: the listener creates a fresh
        // pipe instance / keeps accepting after every connection.
        let second_path = path.clone();
        let second = tokio::spawn(async move {
            let mut stream = connect_at(second_path).await.unwrap();
            send_line(&mut stream, &"pong".to_string()).await.unwrap();
        });

        let stream = listener.accept().await.unwrap();
        let (read_half, _write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let got: Option<String> = read_line(&mut reader).await.unwrap();
        assert_eq!(got.as_deref(), Some("pong"));
        second.await.unwrap();

        drop(listener);
        #[cfg(unix)]
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn ipc_sync_probe_reports_liveness_without_breaking_accept() {
        let path = test_endpoint();
        let mut listener = IpcListener::bind_at(path.clone()).await.unwrap();

        // The launcher's synchronous probe must detect the live endpoint.
        assert!(connect_sync_at(path.clone()));

        // On Windows the probe is a real client connection: accepting it hands
        // back a peer that is already gone. The listener must survive that and
        // still serve the next client.
        drop(listener.accept().await.unwrap());

        let client_path = path.clone();
        let client = tokio::spawn(async move {
            let mut stream = connect_at(client_path).await.unwrap();
            send_line(&mut stream, &"after-probe".to_string())
                .await
                .unwrap();
        });

        let stream = listener.accept().await.unwrap();
        let (read_half, _write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let got: Option<String> = read_line(&mut reader).await.unwrap();
        assert_eq!(got.as_deref(), Some("after-probe"));
        client.await.unwrap();

        drop(listener);
        #[cfg(unix)]
        let _ = std::fs::remove_file(&path);

        // Nothing listening anymore: the probe reports "no daemon".
        assert!(!connect_sync_at(path));
    }
}
