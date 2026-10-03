use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn current_utc_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WallpaperMetadata {
    pub provider: String,
    pub source_name: Option<String>,
    pub changed_at: u64,
    pub opaque_id: String,
    pub source_url: Option<String>,
    pub cache_path: PathBuf,
    pub title: Option<String>,
    pub author: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefetchedWallpaper {
    pub provider: String,
    pub source_name: Option<String>,
    pub opaque_id: String,
    pub source_url: Option<String>,
    pub cache_path: PathBuf,
    pub title: Option<String>,
    pub author: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub prefetched_at: u64,
}

impl From<PrefetchedWallpaper> for WallpaperMetadata {
    fn from(p: PrefetchedWallpaper) -> Self {
        Self {
            provider: p.provider,
            source_name: p.source_name,
            changed_at: current_utc_timestamp(),
            opaque_id: p.opaque_id,
            source_url: p.source_url,
            cache_path: p.cache_path,
            title: p.title,
            author: p.author,
            width: p.width,
            height: p.height,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    /// 0 = currently viewing the latest wallpaper (history[0]).
    /// > 0 = browsing older wallpapers in history.
    pub current_history_index: usize,

    /// History of displayed wallpapers (index 0 is most recent).
    pub history: Vec<WallpaperMetadata>,

    /// Queue of pre-downloaded wallpapers ready to be displayed.
    pub prefetch_queue: Vec<PrefetchedWallpaper>,

    /// Last timestamp UTC when wallpaper was actually changed on desktop.
    pub last_changed_at: Option<u64>,
}

impl State {
    pub fn state_path() -> PathBuf {
        if let Ok(p) = std::env::var("WALLPP_STATE") {
            return PathBuf::from(p);
        }
        let base = std::env::var("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".local").join("state")
            });
        base.join("wallpp").join("state.bin")
    }

    pub fn load_or_default() -> Self {
        let path = Self::state_path();
        if path.exists() {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(state) = bincode::deserialize::<State>(&bytes) {
                    return state;
                }
            }
            // Incompatible format, remove and start fresh.
            let _ = std::fs::remove_file(&path);
        }
        State::default()
    }

    /// Derive the name or provider of the last displayed wallpaper from history.
    pub fn last_source(&self) -> Option<&str> {
        self.history
            .first()
            .map(|h| h.source_name.as_deref().unwrap_or(&h.provider))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create state directory at {:?}", parent))?;
        }

        let encoded = bincode::serialize(self).context("Failed to serialize state with bincode")?;

        // Atomic write: write to temp file then rename.
        let tmp_path = path.with_extension(format!("bin.tmp.{}", std::process::id()));
        std::fs::write(&tmp_path, encoded)
            .with_context(|| format!("Failed to write temporary state file at {:?}", tmp_path))?;

        std::fs::rename(&tmp_path, &path).with_context(|| {
            format!(
                "Failed to atomic rename state file {:?} to {:?}",
                tmp_path, path
            )
        })?;

        Ok(())
    }

    pub fn add_to_history(&mut self, entry: WallpaperMetadata, max_history: usize) {
        self.history.insert(0, entry);
        self.current_history_index = 0;
        self.last_changed_at = Some(current_utc_timestamp());
        if self.history.len() > max_history {
            self.history.truncate(max_history);
        }
    }

    /// Selects an existing history entry and promotes it to a new head entry (index 0).
    /// Updates the timestamp and sets it as the active wallpaper.
    pub fn promote(&mut self, history_idx: usize, max_history: usize) -> Option<WallpaperMetadata> {
        let entry = self.history.get(history_idx)?.clone();
        let mut new_entry = entry;
        new_entry.changed_at = current_utc_timestamp();
        self.add_to_history(new_entry.clone(), max_history);
        Some(new_entry)
    }

    pub fn get_current_wallpaper(&self) -> Option<&WallpaperMetadata> {
        self.history.get(self.current_history_index)
    }

    pub fn step_previous(&mut self) -> Option<&WallpaperMetadata> {
        if self.current_history_index + 1 < self.history.len() {
            self.current_history_index += 1;
            self.history.get(self.current_history_index)
        } else {
            None
        }
    }

    pub fn step_next(&mut self) -> Option<&WallpaperMetadata> {
        if self.current_history_index > 0 {
            self.current_history_index -= 1;
            self.history.get(self.current_history_index)
        } else {
            None
        }
    }

    pub fn pop_prefetched(&mut self, target_source: Option<&str>) -> Option<PrefetchedWallpaper> {
        // Clean up any stale prefetched items whose file was removed.
        self.prefetch_queue.retain(|item| item.cache_path.exists());

        if let Some(source) = target_source {
            let pos = self.prefetch_queue.iter().position(|item| {
                item.source_name.as_deref() == Some(source) || item.provider == source
            });
            if let Some(idx) = pos {
                return Some(self.prefetch_queue.remove(idx));
            }
            None
        } else if !self.prefetch_queue.is_empty() {
            Some(self.prefetch_queue.remove(0))
        } else {
            None
        }
    }
}
