use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::wallpp::provider::types::{
    Config as WitConfig, ConfigEntry, ConfigValue, ScalarType, ScalarValue,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default = "default_sources")]
    pub source: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceConfig {
    pub name: Option<String>,
    pub provider: String,
    #[serde(flatten)]
    pub options: HashMap<String, toml::Value>,
}

fn default_sources() -> Vec<SourceConfig> {
    let mut options = HashMap::new();
    options.insert(
        "subreddits".to_string(),
        toml::Value::Array(vec![toml::Value::String("wallpapers".to_string())]),
    );
    options.insert("sort".to_string(), toml::Value::String("hot".to_string()));

    vec![SourceConfig {
        name: Some("default-reddit".to_string()),
        provider: "reddit".to_string(),
        options,
    }]
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
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config file at {:?}", path))?;
            let cfg: AppConfig = toml::from_str(&content)
                .with_context(|| format!("Failed to parse TOML in {:?}", path))?;
            Ok(cfg)
        } else {
            Ok(AppConfig {
                source: default_sources(),
            })
        }
    }
}

pub fn toml_to_wit_config(
    source_options: &HashMap<String, toml::Value>,
    specs: &[crate::wallpp::provider::types::OptionSpec],
) -> WitConfig {
    let mut entries = Vec::new();

    for spec in specs {
        let val_opt = source_options.get(&spec.key);

        let config_value = match (val_opt, &spec.default) {
            (Some(v), _) => parse_toml_value(v, spec),
            (None, Some(def)) => Some(def.clone()),
            (None, None) => None,
        };

        if let Some(cv) = config_value {
            entries.push(ConfigEntry {
                key: spec.key.clone(),
                value: cv,
            });
        }
    }

    entries
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
                eprintln!("Warning: value '{}' not in allowed choices {:?}", s, allowed);
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
