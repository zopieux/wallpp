//! XDG base-directory resolution shared by config, state, cache, and
//! provider search paths.

use std::path::PathBuf;

fn xdg_base(var: &str, fallback: &[&str]) -> PathBuf {
    std::env::var(var).map(PathBuf::from).unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        fallback
            .iter()
            .fold(PathBuf::from(home), |path, part| path.join(part))
    })
}

pub fn config_dir() -> PathBuf {
    xdg_base("XDG_CONFIG_HOME", &[".config"])
}

pub fn state_dir() -> PathBuf {
    xdg_base("XDG_STATE_HOME", &[".local", "state"])
}

pub fn cache_dir() -> PathBuf {
    xdg_base("XDG_CACHE_HOME", &[".cache"])
}

pub fn data_dir() -> PathBuf {
    xdg_base("XDG_DATA_HOME", &[".local", "share"])
}
