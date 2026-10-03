use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    pub fn new() -> Result<Self> {
        let base = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".cache")
            });
        let cache_dir = base.join("wallpp");
        fs::create_dir_all(cache_dir.join("images"))?;
        Ok(Self { cache_dir })
    }

    pub fn get_cached_image(&self, id: &str) -> Option<PathBuf> {
        let filename = hash_id(id);
        let images_dir = self.cache_dir.join("images");
        for ext in &["jpg", "jpeg", "png", "webp", "bin"] {
            let path = images_dir.join(format!("{}.{}", filename, ext));
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    pub fn store_image(&self, id: &str, content_type: &str, data: &[u8]) -> Result<PathBuf> {
        let filename = hash_id(id);
        let ext = match content_type {
            "image/png" => "png",
            "image/webp" => "webp",
            _ => "jpg",
        };
        let path = self
            .cache_dir
            .join("images")
            .join(format!("{}.{}", filename, ext));
        fs::write(&path, data)
            .with_context(|| format!("Failed to write cached image to {:?}", path))?;
        Ok(path)
    }

    pub fn enforce_max_size(&self, max_bytes: u64, keep_paths: &[PathBuf]) -> Result<()> {
        let images_dir = self.cache_dir.join("images");
        if !images_dir.exists() {
            return Ok(());
        }

        let mut files = Vec::new();
        let mut total_size = 0u64;

        for entry in fs::read_dir(&images_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Ok(meta) = entry.metadata() {
                    let len = meta.len();
                    total_size += len;
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    files.push((path, len, mtime));
                }
            }
        }

        if total_size <= max_bytes {
            return Ok(());
        }

        // Sort by modification time ascending (oldest first).
        files.sort_by_key(|(_, _, mtime)| *mtime);

        for (path, len, _) in files {
            if total_size <= max_bytes {
                break;
            }
            if keep_paths.iter().any(|p| p == &path) {
                continue;
            }
            if let Ok(()) = fs::remove_file(&path) {
                total_size = total_size.saturating_sub(len);
            }
        }

        Ok(())
    }
}

fn hash_id(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}
