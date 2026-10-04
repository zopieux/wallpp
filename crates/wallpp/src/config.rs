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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl std::fmt::Display for ByteSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let b = self.0;
        if b >= 1024 * 1024 * 1024 && b.is_multiple_of(1024 * 1024 * 1024) {
            write!(f, "{}G", b / (1024 * 1024 * 1024))
        } else if b >= 1024 * 1024 && b.is_multiple_of(1024 * 1024) {
            write!(f, "{}M", b / (1024 * 1024))
        } else if b >= 1024 && b.is_multiple_of(1024) {
            write!(f, "{}K", b / 1024)
        } else {
            write!(f, "{}B", b)
        }
    }
}

impl Serialize for ByteSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
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

impl std::str::FromStr for ByteSize {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_byte_size(s).map(ByteSize)
    }
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

fn default_min_display_percentage() -> u32 {
    100
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerConfig {
    #[serde(default)]
    pub cache_max_size: ByteSize,

    #[serde(default = "default_history_length")]
    pub history_length: usize,

    #[serde(default = "default_prefetch_count")]
    pub prefetch_count: usize,

    #[serde(
        default,
        with = "humantime_serde::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub refresh_interval: Option<std::time::Duration>,

    #[serde(default = "default_true")]
    pub refresh_at_boot: bool,

    #[serde(default)]
    pub refresh_strategy: RefreshStrategy,

    #[serde(
        default,
        alias = "source_selection_strategy",
        alias = "selection_strategy"
    )]
    pub source_strategy: SourceStrategy,

    #[serde(
        default = "default_min_display_percentage",
        alias = "min_percentage_of_biggest_display",
        alias = "min_percentage_display",
        alias = "min_display_percent"
    )]
    pub min_display_percentage: u32,
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
            min_display_percentage: default_min_display_percentage(),
        }
    }
}

impl ManagerConfig {
    pub fn refresh_interval_str(&self) -> String {
        self.refresh_interval
            .map(|d| humantime::format_duration(d).to_string())
            .unwrap_or_default()
    }

    pub fn set_refresh_interval_from_str(&mut self, s: &str) -> anyhow::Result<()> {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            self.refresh_interval = None;
            Ok(())
        } else {
            let dur = humantime::parse_duration(trimmed)?;
            self.refresh_interval = Some(dur);
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub manager: ManagerConfig,

    #[serde(default)]
    pub source: Vec<SourceConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SourceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub provider: String,
    #[serde(flatten)]
    pub options: HashMap<String, toml::Value>,
}

impl SourceConfig {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            name: None,
            provider: provider.into(),
            options: HashMap::new(),
        }
    }

    pub fn from_default_config(
        provider: String,
        default_config: &[crate::wallpp::provider::types::ConfigEntry],
    ) -> Self {
        use crate::wallpp::provider::types::{ConfigValue, ScalarValue};
        let mut sc = Self::new(provider);
        for entry in default_config {
            match &entry.value {
                ConfigValue::One(ScalarValue::Text(s)) => sc.set_option_string(&entry.key, s),
                ConfigValue::One(ScalarValue::Choice(s)) => sc.set_option_string(&entry.key, s),
                ConfigValue::One(ScalarValue::Integer(i)) => sc.set_option_int(&entry.key, *i),
                ConfigValue::One(ScalarValue::Boolean(b)) => sc.set_option_bool(&entry.key, *b),
                ConfigValue::Many(items) => {
                    let strings: Vec<String> = items
                        .iter()
                        .map(|it| match it {
                            ScalarValue::Text(s) | ScalarValue::Choice(s) => s.clone(),
                            ScalarValue::Integer(i) => i.to_string(),
                            ScalarValue::Boolean(b) => b.to_string(),
                        })
                        .collect();
                    sc.set_option_string_list(&entry.key, &strings);
                }
            }
        }
        sc
    }

    pub fn get_option_string(&self, key: &str) -> Option<String> {
        match self.options.get(key) {
            Some(toml::Value::String(s)) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn get_option_bool(&self, key: &str) -> Option<bool> {
        match self.options.get(key) {
            Some(toml::Value::Boolean(b)) => Some(*b),
            _ => None,
        }
    }

    pub fn get_option_int(&self, key: &str) -> Option<i64> {
        match self.options.get(key) {
            Some(toml::Value::Integer(i)) => Some(*i),
            _ => None,
        }
    }

    pub fn get_option_string_list(&self, key: &str) -> Vec<String> {
        match self.options.get(key) {
            Some(toml::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| match v {
                    toml::Value::String(s) => Some(s.clone()),
                    toml::Value::Integer(i) => Some(i.to_string()),
                    toml::Value::Boolean(b) => Some(b.to_string()),
                    _ => None,
                })
                .collect(),
            Some(toml::Value::String(s)) => s
                .split(',')
                .map(|it| it.trim().to_string())
                .filter(|it| !it.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn set_option_string(&mut self, key: &str, val: impl Into<String>) {
        let s = val.into();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            self.options.remove(key);
        } else {
            self.options
                .insert(key.to_string(), toml::Value::String(trimmed.to_string()));
        }
    }

    pub fn set_option_bool(&mut self, key: &str, val: bool) {
        self.options
            .insert(key.to_string(), toml::Value::Boolean(val));
    }

    pub fn set_option_int(&mut self, key: &str, val: i64) {
        self.options
            .insert(key.to_string(), toml::Value::Integer(val));
    }

    pub fn set_option_string_list(&mut self, key: &str, vals: &[String]) {
        if vals.is_empty() {
            self.options.remove(key);
        } else {
            let arr = vals
                .iter()
                .map(|s| toml::Value::String(s.clone()))
                .collect();
            self.options
                .insert(key.to_string(), toml::Value::Array(arr));
        }
    }

    pub fn remove_option(&mut self, key: &str) {
        self.options.remove(key);
    }

    pub fn to_wit_config(
        &self,
        specs: &[crate::wallpp::provider::types::OptionSpec],
        default_config: &[ConfigEntry],
    ) -> WitConfig {
        toml_to_wit_config(&self.options, specs, default_config)
    }
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

    /// Loads the configuration file directly without layering environment variables.
    /// Used by the TUI config editor so transient env vars are not persisted to disk.
    pub fn load_file_only() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config file at {:?}", path))?;
            let app_config: AppConfig = toml::from_str(&text)
                .with_context(|| format!("Failed to parse config from {:?}", path))?;
            Ok(app_config)
        } else {
            Ok(AppConfig::default())
        }
    }

    /// Atomically writes the configuration to disk in TOML format.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory at {:?}", parent))?;
        }
        let serialized =
            toml::to_string_pretty(self).context("Failed to serialize configuration to TOML")?;

        let tmp_path = path.with_extension(format!("toml.tmp.{}", std::process::id()));
        std::fs::write(&tmp_path, serialized)
            .with_context(|| format!("Failed to write temporary config file at {:?}", tmp_path))?;
        std::fs::rename(&tmp_path, &path).with_context(|| {
            format!(
                "Failed to atomic rename config file {:?} to {:?}",
                tmp_path, path
            )
        })?;
        Ok(())
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
