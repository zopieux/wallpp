use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

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
        fs::create_dir_all(&cache_dir.join("images"))?;
        fs::create_dir_all(&cache_dir.join("metadata"))?;
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
        let path = self.cache_dir.join("images").join(format!("{}.{}", filename, ext));
        fs::write(&path, data).with_context(|| format!("Failed to write cached image to {:?}", path))?;
        Ok(path)
    }

    #[allow(dead_code)]
    pub fn get_cached_metadata(&self, source_key: &str) -> Option<String> {
        let filename = hash_id(source_key);
        let path = self.cache_dir.join("metadata").join(format!("{}.json", filename));
        if let Ok(metadata) = fs::metadata(&path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(elapsed) = SystemTime::now().duration_since(modified) {
                    if elapsed < Duration::from_secs(600) {
                        return fs::read_to_string(&path).ok();
                    }
                }
            }
        }
        None
    }

    #[allow(dead_code)]
    pub fn store_metadata(&self, source_key: &str, json_content: &str) -> Result<()> {
        let filename = hash_id(source_key);
        let path = self.cache_dir.join("metadata").join(format!("{}.json", filename));
        fs::write(&path, json_content)?;
        Ok(())
    }
}

fn hash_id(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}
