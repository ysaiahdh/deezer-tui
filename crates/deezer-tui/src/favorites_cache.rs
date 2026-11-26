//! Persistent disk cache for favorites data.
//!
//! Avoids re-fetching from the Deezer API every time the user switches
//! between favorites sub-tabs. The cache is stored as a JSON file in the
//! XDG config directory (`~/.config/deezer-tui/favorites_cache.json`).
//!
//! # Invalidation rules
//! - Tracks cache   → invalidated on `FavoriteAdded` / `FavoriteRemoved`
//! - Artists cache  → invalidated on `FavoriteArtistAdded` / `FavoriteArtistRemoved`
//! - Albums cache   → invalidated on `FavoriteAlbumAdded` / `FavoriteAlbumRemoved`
//! - Playlists list → invalidated on `AddedToPlaylist` (playlist list may have changed)
//! - Playlist detail → invalidated for the specific playlist on `AddedToPlaylist`
//! - RecentlyPlayed → never cached (changes after every track play)
//! - Following      → invalidated explicitly (no in-app mutation currently)
//!
//! # Staleness
//! In-app mutations are not the only way favorites change: they can also be
//! added from the web player or the mobile app, which this process never sees.
//! Every section therefore records when it was last synced from the API
//! ([`FavoritesCache::mark_synced`]); the daemon serves the cached copy
//! instantly and revalidates it in the background once it is older than
//! [`REVALIDATE_AFTER_SECS`] ([`FavoritesCache::needs_revalidation`]).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use deezer_core::api::models::{DisplayItem, PlaylistDetail, TrackData};
use deezer_core::Config;

use crate::protocol::MoodEntry;

/// Current cache format. Bump to discard every on-disk cache written by an
/// older build — used when a bug could have stored wrong data under a key.
/// 3: track counts are stored as a plain number, not "<N> titres" (#18).
const CACHE_VERSION: u32 = 3;

/// How long a cached section is served without asking the API again. Past
/// that, the cached copy is still shown at once but revalidated in the
/// background.
pub const REVALIDATE_AFTER_SECS: u64 = 5 * 60;

/// Unix time in seconds, 0 if the clock is before the epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// All cached favorites data, persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FavoritesCache {
    /// Format version of this cache file; mismatched files are discarded.
    #[serde(default)]
    pub version: u32,
    /// Favorite tracks (FavoritesCategory::Tracks).
    #[serde(default)]
    pub tracks: Option<Vec<TrackData>>,
    /// Favorite artists as display items (FavoritesCategory::Artists).
    #[serde(default)]
    pub artists: Option<Vec<DisplayItem>>,
    /// Favorite albums as display items (FavoritesCategory::Albums).
    #[serde(default)]
    pub albums: Option<Vec<DisplayItem>>,
    /// User playlists as display items (FavoritesCategory::Playlists).
    #[serde(default)]
    pub playlists: Option<Vec<DisplayItem>>,
    /// Following as display items (FavoritesCategory::Following).
    #[serde(default)]
    pub following: Option<Vec<DisplayItem>>,
    /// Individual playlist details, keyed by playlist_id.
    #[serde(default)]
    pub playlist_details: HashMap<String, PlaylistDetail>,
    /// Explore > Moods list. Rarely changes — cached forever, refreshed on login.
    #[serde(default)]
    pub moods: Option<Vec<MoodEntry>>,
    /// Unix timestamp of the last successful sync from the API, per section
    /// (`tracks`, `artists`, …). Missing means "never synced", so the section
    /// is revalidated on first use — including for files written by an older
    /// build, which had no way to tell.
    #[serde(default)]
    pub synced_at: HashMap<String, u64>,
}

/// Unambiguous fingerprint of a display-item list: order sensitive, and it
/// includes the count column so a playlist's track count changing on another
/// device counts as a change.
fn display_signature(items: &[DisplayItem]) -> String {
    let mut signature = String::new();
    for item in items {
        for field in [&item.col1, &item.col2, &item.col3, &item.col4] {
            signature.push_str(field);
            signature.push('\u{1f}');
        }
    }
    signature
}

impl FavoritesCache {
    // ── Section names (keys of `synced_at`) ──────────────────────────────
    pub const TRACKS: &'static str = "tracks";
    pub const ARTISTS: &'static str = "artists";
    pub const ALBUMS: &'static str = "albums";
    pub const PLAYLISTS: &'static str = "playlists";
    pub const FOLLOWING: &'static str = "following";

    /// File path: `<config_dir>/favorites_cache.json`.
    fn path() -> Option<std::path::PathBuf> {
        Config::dir().map(|d| d.join("favorites_cache.json"))
    }

    /// An empty cache stamped with the current format version.
    fn empty() -> Self {
        Self {
            version: CACHE_VERSION,
            ..Self::default()
        }
    }

    /// Load from disk. Returns an empty cache on any error, or when the file
    /// was written by an older, incompatible cache format.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::empty();
        };
        let cache = match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str::<Self>(&content).unwrap_or_default(),
            Err(_) => return Self::empty(),
        };
        if cache.version == CACHE_VERSION {
            cache
        } else {
            Self::empty()
        }
    }

    /// Persist to disk (best-effort; errors are silently ignored).
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut to_write = self.clone();
        to_write.version = CACHE_VERSION;
        if let Ok(content) = serde_json::to_string(&to_write) {
            let _ = std::fs::write(path, content);
        }
    }

    // ── Staleness ─────────────────────────────────────────────────────────

    /// Record `section` as just synced from the API. Call it **after** storing
    /// a freshly fetched list, never before.
    pub fn mark_synced(&mut self, section: &str) {
        self.synced_at.insert(section.to_string(), unix_now());
    }

    /// Whether `section` must be fetched again to catch up with favorites
    /// changed elsewhere: unknown (never synced, or written by an older build)
    /// or older than [`REVALIDATE_AFTER_SECS`].
    pub fn needs_revalidation(&self, section: &str) -> bool {
        match self.synced_at.get(section) {
            Some(&synced_at) => unix_now().saturating_sub(synced_at) >= REVALIDATE_AFTER_SECS,
            None => true,
        }
    }

    /// Mark `section` as stale so the next load revalidates it, without
    /// dropping the data: a failed refresh keeps the last good list on screen.
    pub fn mark_stale(&mut self, section: &str) {
        self.synced_at.remove(section);
    }

    /// Cached display items of a section (`artists`, `albums`, …), if any.
    pub fn display_items(&self, section: &str) -> Option<&[DisplayItem]> {
        match section {
            Self::ARTISTS => self.artists.as_deref(),
            Self::ALBUMS => self.albums.as_deref(),
            Self::PLAYLISTS => self.playlists.as_deref(),
            Self::FOLLOWING => self.following.as_deref(),
            _ => None,
        }
    }

    /// Store display items for `section`, returning whether they changed.
    pub fn set_display_items(&mut self, section: &str, items: Vec<DisplayItem>) -> bool {
        let changed = self
            .display_items(section)
            .is_none_or(|cached| display_signature(cached) != display_signature(&items));
        match section {
            Self::ARTISTS => self.artists = Some(items),
            Self::ALBUMS => self.albums = Some(items),
            Self::PLAYLISTS => self.playlists = Some(items),
            Self::FOLLOWING => self.following = Some(items),
            _ => return false,
        }
        self.mark_synced(section);
        changed
    }

    // ── Invalidation helpers ──────────────────────────────────────────────

    pub fn invalidate_tracks(&mut self) {
        self.tracks = None;
        self.mark_stale(Self::TRACKS);
    }

    pub fn invalidate_artists(&mut self) {
        self.artists = None;
        self.mark_stale(Self::ARTISTS);
    }

    pub fn invalidate_albums(&mut self) {
        self.albums = None;
        self.mark_stale(Self::ALBUMS);
    }

    /// Invalidate the playlists list and, optionally, a specific playlist's detail.
    pub fn invalidate_playlists(&mut self, playlist_id: Option<&str>) {
        self.playlists = None;
        self.mark_stale(Self::PLAYLISTS);
        if let Some(id) = playlist_id {
            self.playlist_details.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_section_needs_revalidation() {
        let cache = FavoritesCache::default();
        assert!(cache.needs_revalidation(FavoritesCache::TRACKS));
    }

    #[test]
    fn fresh_section_is_served_without_a_network_call() {
        let mut cache = FavoritesCache::default();
        cache.mark_synced(FavoritesCache::TRACKS);
        assert!(!cache.needs_revalidation(FavoritesCache::TRACKS));
        // Other sections are unaffected.
        assert!(cache.needs_revalidation(FavoritesCache::ALBUMS));
    }

    #[test]
    fn old_section_needs_revalidation() {
        let mut cache = FavoritesCache::default();
        cache.synced_at.insert(
            FavoritesCache::TRACKS.to_string(),
            unix_now().saturating_sub(REVALIDATE_AFTER_SECS + 1),
        );
        assert!(cache.needs_revalidation(FavoritesCache::TRACKS));
    }

    #[test]
    fn invalidation_forgets_the_sync_stamp() {
        let mut cache = FavoritesCache::default();
        cache.mark_synced(FavoritesCache::ARTISTS);
        cache.invalidate_artists();
        assert!(cache.artists.is_none());
        assert!(cache.needs_revalidation(FavoritesCache::ARTISTS));
    }

    #[test]
    fn cache_without_stamps_still_parses() {
        // Files written by an older build have no `synced_at` field; they must
        // load (and be revalidated) instead of being discarded.
        let cache: FavoritesCache = serde_json::from_str(
            r#"{"version":3,"tracks":[],"artists":null,"albums":null,"playlists":null,
                "following":null,"playlist_details":{},"moods":null}"#,
        )
        .expect("older cache file must still deserialize");
        assert!(cache.needs_revalidation(FavoritesCache::TRACKS));
    }
}
