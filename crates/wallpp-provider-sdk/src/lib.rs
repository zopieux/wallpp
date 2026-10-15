wit_bindgen::generate!({
    path: "../../wit",
    world: "wallpaper-provider",
    pub_export_macro: true,
});
pub use exports::wallpp::provider::provider::Guest;
pub use wallpp::provider::types::*;

pub struct ConfigReader<'a> {
    config: &'a Config,
}

impl<'a> ConfigReader<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    fn one(&self, key: &str) -> Option<&'a ScalarValue> {
        self.config.iter().find_map(|entry| {
            if entry.key == key {
                if let ConfigValue::One(ref value) = entry.value {
                    return Some(value);
                }
            }
            None
        })
    }

    fn many(&self, key: &str) -> Option<&'a [ScalarValue]> {
        self.config.iter().find_map(|entry| {
            if entry.key == key {
                if let ConfigValue::Many(ref items) = entry.value {
                    return Some(items.as_slice());
                }
            }
            None
        })
    }

    pub fn get_text(&self, key: &str) -> Option<&'a str> {
        match self.one(key) {
            Some(ScalarValue::Text(s)) => Some(s),
            _ => None,
        }
    }

    pub fn get_choice(&self, key: &str) -> Option<&'a str> {
        match self.one(key) {
            Some(ScalarValue::Choice(s)) => Some(s),
            _ => None,
        }
    }

    pub fn get_integer(&self, key: &str) -> Option<i64> {
        match self.one(key) {
            Some(ScalarValue::Integer(i)) => Some(*i),
            _ => None,
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.one(key) {
            Some(ScalarValue::Boolean(b)) => Some(*b),
            _ => None,
        }
    }

    pub fn get_text_list(&self, key: &str) -> Vec<&'a str> {
        self.many(key)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| match item {
                        ScalarValue::Text(s) | ScalarValue::Choice(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Convert string literals to owned strings.
pub fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// Builder for a provider option specification.
pub struct Spec(OptionSpec);

pub fn spec(key: &str, label: &str, ty: ScalarType) -> Spec {
    Spec(OptionSpec {
        key: key.to_string(),
        label: label.to_string(),
        description: None,
        ty,
        multiple: false,
        required: false,
        default: None,
    })
}

impl Spec {
    pub fn desc(mut self, description: &str) -> Self {
        self.0.description = Some(description.to_string());
        self
    }

    pub fn multiple(mut self) -> Self {
        self.0.multiple = true;
        self
    }

    pub fn required(mut self) -> Self {
        self.0.required = true;
        self
    }

    pub fn default_text(mut self, value: &str) -> Self {
        self.0.default = Some(ConfigValue::One(ScalarValue::Text(value.to_string())));
        self
    }

    pub fn default_choice(mut self, value: &str) -> Self {
        self.0.default = Some(ConfigValue::One(ScalarValue::Choice(value.to_string())));
        self
    }

    pub fn default_bool(mut self, value: bool) -> Self {
        self.0.default = Some(ConfigValue::One(ScalarValue::Boolean(value)));
        self
    }

    pub fn default_int(mut self, value: i64) -> Self {
        self.0.default = Some(ConfigValue::One(ScalarValue::Integer(value)));
        self
    }

    pub fn default_texts(mut self, values: &[&str]) -> Self {
        self.0.default = Some(ConfigValue::Many(
            values
                .iter()
                .map(|value| ScalarValue::Text(value.to_string()))
                .collect(),
        ));
        self
    }

    pub fn default_choices(mut self, values: &[&str]) -> Self {
        self.0.default = Some(ConfigValue::Many(
            values
                .iter()
                .map(|value| ScalarValue::Choice(value.to_string()))
                .collect(),
        ));
        self
    }

    pub fn build(self) -> OptionSpec {
        self.0
    }
}

/// Build provider info, deriving the default config from selected option defaults.
pub fn provider_info(
    name: &str,
    label: &str,
    version: &str,
    options: Vec<OptionSpec>,
    default_keys: &[&str],
    allowed_hosts: &[&str],
) -> ProviderInfo {
    let default_config = options
        .iter()
        .filter(|option| default_keys.contains(&option.key.as_str()))
        .filter_map(|option| {
            option.default.clone().map(|value| ConfigEntry {
                key: option.key.clone(),
                value,
            })
        })
        .collect();
    ProviderInfo {
        name: name.to_string(),
        label: label.to_string(),
        version: version.to_string(),
        options,
        default_config,
        allowed_hosts: strings(allowed_hosts),
    }
}

#[cfg(feature = "wstd")]
pub async fn http_get(
    url: &str,
    headers: &[(&str, &str)],
    rate_limit_exhausted: impl FnOnce(&wstd::http::HeaderMap) -> bool,
) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    use wstd::io::AsyncRead as _;

    let mut builder = wstd::http::Request::get(url);
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    let request = builder
        .body(wstd::io::empty())
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    let mut response = wstd::http::Client::new()
        .send(request)
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    let status = response.status();
    let exhausted = status.as_u16() == 403 && rate_limit_exhausted(response.headers());
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    if status.is_success() {
        return Ok((body, content_type));
    }
    let message = String::from_utf8_lossy(&body);
    let message = message.trim();
    if status.as_u16() == 429 || exhausted || message.to_lowercase().contains("rate limit") {
        return Err(ProviderError::RateLimited(None));
    }
    let error = if message.is_empty() {
        format!("HTTP {}", status)
    } else {
        format!("HTTP {}: {}", status, message)
    };
    Err(match status.as_u16() {
        401 | 403 => ProviderError::Auth(error),
        _ => ProviderError::Network(error),
    })
}
