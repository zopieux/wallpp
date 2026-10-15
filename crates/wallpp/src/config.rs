use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::wallpp::provider::types::{
    Config as WitConfig, ConfigEntry, ConfigValue, ScalarType, ScalarValue,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RefreshStrategy {
    #[default]
    FromBoot,
    FromLastChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceStrategy {
    #[default]
    Random,
    #[serde(alias = "roundrobin")]
    RoundRobin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ByteSize(pub u64);

impl ByteSize {
    pub fn as_bytes(&self) -> u64 {
        self.0
    }
}

impl Default for ByteSize {
    fn default() -> Self {
        ByteSize(1024 * 1024 * 1024) // 1 GiB.
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ByteSizeVisitor;

        impl<'de> serde::de::Visitor<'de> for ByteSizeVisitor {
            type Value = ByteSize;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(
                    "a byte size integer or human-readable string like '1G', '500M', '2GiB'",
                )
            }

            fn visit_i64<E>(self, v: i64) -> Result<ByteSize, E>
            where
                E: serde::de::Error,
            {
                if v < 0 {
                    Err(E::custom("byte size cannot be negative"))
                } else {
                    Ok(ByteSize(v as u64))
                }
            }

            fn visit_u64<E>(self, v: u64) -> Result<ByteSize, E>
            where
                E: serde::de::Error,
            {
                Ok(ByteSize(v))
            }

            fn visit_str<E>(self, v: &str) -> Result<ByteSize, E>
            where
                E: serde::de::Error,
            {
                parse_byte_size(v).map(ByteSize).map_err(E::custom)
            }
        }

        deserializer.deserialize_any(ByteSizeVisitor)
    }
}

fn parse_byte_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty byte size".to_string());
    }

    let mut num_end = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() || c == '.' {
            num_end = i + c.len_utf8();
        } else {
            break;
        }
    }

    if num_end == 0 {
        return Err(format!("invalid byte size '{}'", s));
    }

    let (num_str, unit_str) = s.split_at(num_end);
    let val: f64 = num_str
        .parse()
        .map_err(|e| format!("invalid number in byte size '{}': {}", s, e))?;

    let unit = unit_str.trim().to_uppercase();
    let multiplier: u64 = match unit.as_str() {
        "" | "B" => 1,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "G" | "GB" | "GIB" => 1024 * 1024 * 1024,
        "T" | "TB" | "TIB" => 1024 * 1024 * 1024 * 1024,
        _ => return Err(format!("unknown unit '{}' in byte size", unit)),
    };

    Ok((val * multiplier as f64) as u64)
}

fn default_true() -> bool {
    true
}

fn default_history_length() -> usize {
    1000
}

fn default_prefetch_count() -> usize {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerConfig {
    #[serde(default)]
    pub cache_max_size: ByteSize,

    #[serde(default = "default_history_length")]
    pub history_length: usize,

    #[serde(default = "default_prefetch_count")]
    pub prefetch_count: usize,

    #[serde(default)]
    pub refresh_interval: Option<u64>,

    #[serde(default = "default_true")]
    pub refresh_at_boot: bool,

    #[serde(default)]
    pub refresh_strategy: RefreshStrategy,

    #[serde(default, alias = "source_selection_strategy", alias = "selection_strategy")]
    pub source_strategy: SourceStrategy,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            cache_max_size: ByteSize::default(),
            history_length: default_history_length(),
            prefetch_count: default_prefetch_count(),
            refresh_interval: None,
            refresh_at_boot: true,
            refresh_strategy: RefreshStrategy::default(),
            source_strategy: SourceStrategy::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub manager: ManagerConfig,

    #[serde(default)]
    pub source: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceConfig {
    pub name: Option<String>,
    pub provider: String,
    #[serde(flatten)]
    pub options: HashMap<String, toml::Value>,
}

impl AppConfig {
    pub fn config_path() -> PathBuf {
        if let Ok(p) = std::env::var("WALLPP_CONFIG") {
            return PathBuf::from(p);
        }
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".config")
            });
        base.join("wallpp").join("config.toml")
    }

    pub fn load_or_default() -> Result<Self> {
        let path = Self::config_path();
        let mut builder = config::Config::builder();

        if path.exists() {
            builder = builder
                .add_source(config::File::from(path.clone()).format(config::FileFormat::Toml));
        }

        builder = builder.add_source(
            config::Environment::with_prefix("WALLPP")
                .separator("__")
                .ignore_empty(true),
        );

        match builder.build() {
            Ok(c) => {
                let app_config: AppConfig = c
                    .try_deserialize()
                    .with_context(|| format!("Failed to parse config from {:?}", path))?;
                Ok(app_config)
            }
            Err(_) if !path.exists() => Ok(AppConfig::default()),
            Err(e) => Err(anyhow::anyhow!("Failed to build configuration: {}", e)),
        }
    }
}

pub fn toml_to_wit_config(
    source_options: &HashMap<String, toml::Value>,
    specs: &[crate::wallpp::provider::types::OptionSpec],
    default_config: &[ConfigEntry],
) -> WitConfig {
    let mut config_map: HashMap<String, ConfigValue> = HashMap::new();
    for entry in default_config {
        config_map.insert(entry.key.clone(), entry.value.clone());
    }

    for spec in specs {
        if let Some(v) = source_options.get(&spec.key) {
            if let Some(cv) = parse_toml_value(v, spec) {
                config_map.insert(spec.key.clone(), cv);
            }
        } else if !config_map.contains_key(&spec.key) {
            if let Some(ref def) = spec.default {
                config_map.insert(spec.key.clone(), def.clone());
            }
        }
    }

    config_map
        .into_iter()
        .map(|(key, value)| ConfigEntry { key, value })
        .collect()
}

fn parse_toml_value(
    val: &toml::Value,
    spec: &crate::wallpp::provider::types::OptionSpec,
) -> Option<ConfigValue> {
    if spec.multiple {
        let mut scalars = Vec::new();
        match val {
            toml::Value::Array(arr) => {
                for item in arr {
                    if let Some(s) = parse_scalar(item, &spec.ty) {
                        scalars.push(s);
                    }
                }
            }
            single => {
                if let Some(s) = parse_scalar(single, &spec.ty) {
                    scalars.push(s);
                }
            }
        }
        Some(ConfigValue::Many(scalars))
    } else {
        parse_scalar(val, &spec.ty).map(ConfigValue::One)
    }
}

fn parse_scalar(val: &toml::Value, ty: &ScalarType) -> Option<ScalarValue> {
    match (val, ty) {
        (toml::Value::String(s), ScalarType::Text | ScalarType::Secret) => {
            Some(ScalarValue::Text(s.clone()))
        }
        (toml::Value::String(s), ScalarType::Choice(allowed)) => {
            if allowed.contains(s) {
                Some(ScalarValue::Choice(s.clone()))
            } else {
                eprintln!(
                    "Warning: value '{}' not in allowed choices {:?}",
                    s, allowed
                );
                None
            }
        }
        (toml::Value::Integer(i), ScalarType::Integer(bounds)) => {
            if let Some(min) = bounds.min {
                if *i < min {
                    return None;
                }
            }
            if let Some(max) = bounds.max {
                if *i > max {
                    return None;
                }
            }
            Some(ScalarValue::Integer(*i))
        }
        (toml::Value::Boolean(b), ScalarType::Boolean) => Some(ScalarValue::Boolean(*b)),
        _ => None,
    }
}
