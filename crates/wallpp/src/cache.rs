use anyhow::{Context, Result};
use image::{DynamicImage, ImageFormat};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Maximum dimensions of the reduced-resolution copy used by grid views.
/// The original remains untouched for the inspector and desktop wallpaper.
const THUMBNAIL_MAX_WIDTH: u32 = 640;
const THUMBNAIL_MAX_HEIGHT: u32 = 360;

#[derive(Clone)]
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    pub fn new() -> Result<Self> {
        Self::with_cache_dir(crate::paths::cache_dir().join("wallpp"))
    }

    fn with_cache_dir(cache_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(cache_dir.join("images"))?;
        fs::create_dir_all(cache_dir.join("thumbnails"))?;
        Ok(Self { cache_dir })
    }

    /// Deterministic location of the reduced-resolution copy of a cached
    /// original. The thumbnail is a JPEG named after the original file stem.
    pub fn thumbnail_path(&self, original: &Path) -> PathBuf {
        self.cache_dir
            .join("thumbnails")
            .join(thumbnail_file_name(original))
    }

    /// Return the thumbnail for `original`, generating it on first use.
    /// Falls back to the original when it cannot be decoded or encoded;
    /// generation is deliberately best-effort so a bad image cannot stop
    /// the original wallpaper from being used.
    pub fn ensure_thumbnail(&self, original: &Path) -> PathBuf {
        if !original.is_file() {
            return original.to_path_buf();
        }
        let thumbnail = self.thumbnail_path(original);
        if thumbnail.is_file() {
            return thumbnail;
        }
        match image::open(original) {
            Ok(img) => match save_thumbnail(&img, &thumbnail) {
                Ok(()) => thumbnail,
                Err(err) => {
                    eprintln!(
                        "Warning: failed to create thumbnail for {:?}: {err}",
                        original
                    );
                    original.to_path_buf()
                }
            },
            Err(err) => {
                eprintln!(
                    "Warning: failed to decode {:?} for thumbnail: {err}",
                    original
                );
                original.to_path_buf()
            }
        }
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
        // Generate the grid copy while the decoded bytes are already in
        // memory. Best-effort: unsupported/corrupt data still leaves the
        // original usable by the inspector and wallpaper setter.
        if let Ok(image) = image::load_from_memory(data) {
            let thumbnail = self.thumbnail_path(&path);
            if let Err(err) = save_thumbnail(&image, &thumbnail) {
                eprintln!("Warning: failed to create thumbnail for {:?}: {err}", path);
            }
        }
        Ok(path)
    }

    pub fn enforce_max_size(&self, max_bytes: u64, keep_paths: &[PathBuf]) -> Result<()> {
        let keep: HashSet<PathBuf> = keep_paths.iter().cloned().collect();
        let keep_stems: HashSet<String> = keep_paths
            .iter()
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
            .collect();
        let mut files = Vec::new();
        let mut total_size = 0u64;

        for dir_name in ["images", "thumbnails"] {
            let dir = self.cache_dir.join(dir_name);
            if !dir.exists() {
                continue;
            }
            for entry in fs::read_dir(&dir)? {
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
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let is_thumbnail = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                == Some("thumbnails");
            if keep.contains(&path) || (is_thumbnail && keep_stems.contains(stem)) {
                continue;
            }
            if let Ok(()) = fs::remove_file(&path) {
                total_size = total_size.saturating_sub(len);
            }
        }

        Ok(())
    }
}

/// Derive the thumbnail location for an original stored in the standard
/// cache layout (`.../images/<file>` -> `.../thumbnails/<stem>.jpg`).
/// This lets UI code find the reduced copy without persisting a second,
/// derivable path in wallpaper state.
pub fn thumbnail_path_for_original(original: &Path) -> PathBuf {
    let images_dir = original.parent().unwrap_or_else(|| Path::new(""));
    let cache_dir = images_dir.parent().unwrap_or(images_dir);
    cache_dir
        .join("thumbnails")
        .join(thumbnail_file_name(original))
}

fn thumbnail_file_name(original: &Path) -> String {
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    format!("{stem}.jpg")
}

fn save_thumbnail(image: &DynamicImage, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let thumbnail = image.thumbnail(THUMBNAIL_MAX_WIDTH, THUMBNAIL_MAX_HEIGHT);
    thumbnail
        .to_rgb8()
        .save_with_format(path, ImageFormat::Jpeg)
        .with_context(|| format!("Failed to write thumbnail to {:?}", path))
}

fn hash_id(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::io::Cursor;

    fn test_cache(name: &str) -> (CacheManager, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("wallpp-cache-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cache = CacheManager::with_cache_dir(dir.clone()).unwrap();
        (cache, dir)
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::new_rgb8(width, height);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn store_image_creates_reduced_thumbnail() {
        let (cache, dir) = test_cache("store");
        let original = cache
            .store_image("wallpaper-1", "image/png", &png_bytes(1200, 600))
            .unwrap();
        let thumbnail = cache.thumbnail_path(&original);

        assert!(original.is_file());
        assert!(thumbnail.is_file());
        assert_eq!(image::open(&thumbnail).unwrap().dimensions(), (640, 320));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ensure_thumbnail_backfills_existing_original() {
        let (cache, dir) = test_cache("backfill");
        let original = cache.cache_dir.join("images").join("existing.png");
        image::DynamicImage::new_rgb8(900, 900)
            .save_with_format(&original, ImageFormat::Png)
            .unwrap();

        let thumbnail = cache.ensure_thumbnail(&original);
        assert_eq!(thumbnail, cache.thumbnail_path(&original));
        assert_eq!(image::open(&thumbnail).unwrap().dimensions(), (360, 360));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn quota_keeps_thumbnail_of_kept_original() {
        let (cache, dir) = test_cache("quota");
        let original = cache
            .store_image("wallpaper-keep", "image/png", &png_bytes(1200, 600))
            .unwrap();
        let thumbnail = cache.thumbnail_path(&original);
        let other = cache
            .store_image("wallpaper-drop", "image/png", &png_bytes(1200, 600))
            .unwrap();

        cache
            .enforce_max_size(1, std::slice::from_ref(&original))
            .unwrap();
        assert!(original.is_file());
        assert!(thumbnail.is_file());
        assert!(!other.exists());

        let _ = fs::remove_dir_all(dir);
    }
}
