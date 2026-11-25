use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::api::models::{AudioQuality, DeezerError};
use crate::player::eq::EqSettings;

const APP_QUALIFIER: &str = "com";
const APP_ORGANIZATION: &str = "deezer-tui";
const APP_NAME: &str = "deezer-tui";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub arl: Option<String>,
    #[serde(default = "default_quality")]
    pub quality: AudioQuality,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub skip_update_check: bool,
    /// Background transparency in percent (0–100, steps of 10). 0 = opaque, 100 = fully transparent.
    #[serde(default)]
    pub bg_transparency: u8,
    /// Enable vim-style navigation keys (h, j, k, l). Disabled by default.
    #[serde(default)]
    pub vim_keys: bool,
    /// Default track order in the Favorites and Playlist views: `"date_desc"`
    /// (newest first, the default), `"date_asc"`, or `"api"` for the order the
    /// API returned (a playlist's own order).
    #[serde(default, deserialize_with = "deserialize_sort_order")]
    pub sort_order: SortOrder,
    /// Graphic equalizer settings (disabled by default).
    #[serde(default)]
    pub equalizer: EqSettings,
}

/// Display order of a track list.
///
/// `DATE_ADD` is what Deezer's web player sorts favourites on; playlists have
/// their own order, which is what the API returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// Most recently added first.
    #[default]
    DateDesc,
    /// Oldest first.
    DateAsc,
    /// Leave the order the API sent.
    Api,
}

impl SortOrder {
    /// Value used in `config.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DateDesc => "date_desc",
            Self::DateAsc => "date_asc",
            Self::Api => "api",
        }
    }
}

/// Read `sort_order` leniently: an unknown or malformed value falls back to the
/// default instead of failing the whole config parse, which would make `load`
/// return a default config and the next save drop the stored ARL.
fn deserialize_sort_order<'de, D>(deserializer: D) -> Result<SortOrder, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(match raw.as_str() {
        Some("date_asc") => SortOrder::DateAsc,
        Some("api") => SortOrder::Api,
        _ => SortOrder::DateDesc,
    })
}

fn default_quality() -> AudioQuality {
    AudioQuality::Mp3_128
}

fn default_volume() -> f32 {
    0.8
}

impl Default for Config {
    fn default() -> Self {
        Self {
            arl: None,
            quality: default_quality(),
            volume: default_volume(),
            theme: None,
            language: None,
            skip_update_check: false,
            bg_transparency: 0,
            vim_keys: false,
            sort_order: SortOrder::default(),
            equalizer: EqSettings::default(),
        }
    }
}

impl Config {
    /// Get the config directory path (XDG on Linux, AppData on Windows, etc.)
    pub fn dir() -> Option<PathBuf> {
        ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
            .map(|p| p.config_dir().to_path_buf())
    }

    /// Get the local data directory (XDG data on Linux, AppData on Windows, etc.)
    /// Used for offline track storage.
    pub fn data_dir() -> Option<PathBuf> {
        ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
            .map(|p| p.data_local_dir().to_path_buf())
    }

    /// Cache directory (XDG cache on Linux). Holds the anonymous files that
    /// back tracks while they stream.
    pub fn cache_dir() -> Option<PathBuf> {
        ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
            .map(|p| p.cache_dir().to_path_buf())
    }

    /// Full path to the config file.
    pub fn path() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("config.json"))
    }

    /// Load config from disk, or return default if not found.
    pub fn load() -> Self {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => Self::default(),
        }
    }

    /// Save config to disk.
    pub fn save(&self) -> Result<(), DeezerError> {
        self.save_to(&Self::config_path()?)
    }

    /// Change some settings on disk: reload the file, apply `edit`, save.
    ///
    /// The daemon and the client both write `config.json`, each its own
    /// fields (volume, quality… for the daemon; theme, language… for the
    /// client). Saving a copy loaded earlier would put back the other
    /// process's stale values, so every change goes through the file.
    pub fn update(edit: impl FnOnce(&mut Config)) -> Result<Config, DeezerError> {
        Self::update_at(&Self::config_path()?, edit)
    }

    fn config_path() -> Result<PathBuf, DeezerError> {
        Self::path().ok_or_else(|| DeezerError::Api("Could not determine config directory".into()))
    }

    fn load_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn update_at(path: &Path, edit: impl FnOnce(&mut Config)) -> Result<Config, DeezerError> {
        let mut config = Self::load_from(path);
        edit(&mut config);
        config.save_to(path)?;
        Ok(config)
    }

    /// Write through a temporary file renamed over the config: a reader in the
    /// other process never sees a truncated file (which would parse as the
    /// default config, and lose the ARL on its next save).
    fn save_to(&self, path: &Path) -> Result<(), DeezerError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| DeezerError::Api(format!("Failed to create config dir: {e}")))?;
        }

        let content = serde_json::to_string_pretty(self)
            .map_err(|e| DeezerError::Api(format!("Failed to serialize config: {e}")))?;

        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&tmp, content)
            .and_then(|()| fs::rename(&tmp, path))
            .map_err(|e| {
                let _ = fs::remove_file(&tmp);
                DeezerError::Api(format!("Failed to write config: {e}"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deezer-tui-config-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir.join("config.json")
    }

    #[test]
    fn update_keeps_fields_written_by_another_process() {
        let path = temp_config("update");
        // The daemon loaded its copy at startup…
        let daemon_copy = Config::default();
        daemon_copy.save_to(&path).unwrap();

        // …then the client changed a setting of its own…
        Config::update_at(&path, |c| c.vim_keys = true).unwrap();

        // …and the daemon now saves the volume: the client's change survives.
        let saved = Config::update_at(&path, |c| c.volume = 0.3).unwrap();
        assert!(saved.vim_keys);
        assert_eq!(saved.volume, 0.3);

        let on_disk = Config::load_from(&path);
        assert!(on_disk.vim_keys);
        assert_eq!(on_disk.volume, 0.3);
        assert!(
            !daemon_copy.vim_keys,
            "the stale copy is not what got saved"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn save_leaves_no_temporary_file_behind() {
        let path = temp_config("atomic");
        Config::default().save_to(&path).unwrap();
        Config::update_at(&path, |c| c.arl = Some("arl".into())).unwrap();

        let files: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(files, ["config.json"]);
        assert_eq!(Config::load_from(&path).arl.as_deref(), Some("arl"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn sort_order_defaults_to_newest_first_and_round_trips() {
        // A config written by an older build has no `sort_order` key: it must
        // load as the new default rather than failing or picking `Api`.
        let path = temp_config("sort-order");
        let dir = path.parent().unwrap();
        fs::create_dir_all(dir).unwrap();
        fs::write(&path, r#"{"volume":0.5}"#).unwrap();

        let config = Config::load_from(&path);
        assert_eq!(config.sort_order, SortOrder::DateDesc);
        assert_eq!(config.sort_order.as_str(), "date_desc");

        for (text, expected) in [
            ("\"date_desc\"", SortOrder::DateDesc),
            ("\"date_asc\"", SortOrder::DateAsc),
            ("\"api\"", SortOrder::Api),
        ] {
            fs::write(&path, format!(r#"{{"sort_order":{text}}}"#)).unwrap();
            assert_eq!(Config::load_from(&path).sort_order, expected);
        }

        // Unknown values fall back to the default instead of refusing to start,
        // and do not cost the rest of the config (the ARL cannot be re-derived).
        fs::write(
            &path,
            r#"{"arl":"secret","sort_order":"sideways","volume":0.25}"#,
        )
        .unwrap();
        let config = Config::load_from(&path);
        assert_eq!(config.sort_order, SortOrder::DateDesc);
        assert_eq!(config.arl.as_deref(), Some("secret"));
        assert_eq!(config.volume, 0.25);

        // Nor does a value of the wrong JSON type.
        fs::write(&path, r#"{"arl":"secret","sort_order":true}"#).unwrap();
        let config = Config::load_from(&path);
        assert_eq!(config.sort_order, SortOrder::DateDesc);
        assert_eq!(config.arl.as_deref(), Some("secret"));

        let _ = fs::remove_dir_all(dir);
    }
}
