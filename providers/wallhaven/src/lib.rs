use serde::Deserialize;
use url::Url;
use wallpp_provider_sdk::*;
use wstd::http::{Client, Request};
use wstd::io::AsyncRead;
use wstd::runtime::block_on;

const USER_AGENT: &str = "wallpp/0.1.0";

#[derive(Deserialize)]
struct WallhavenSearchResponse {
    data: Vec<WallhavenItem>,
}

#[derive(Deserialize)]
struct WallhavenItem {
    id: String,
    url: String,
    path: String,
    dimension_x: u32,
    dimension_y: u32,
    source: Option<String>,
}

async fn http_get(
    url: String,
    referer: Option<String>,
    api_key: Option<String>,
) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    let mut builder = Request::get(&url)
        .header("user-agent", USER_AGENT)
        .header("accept", "*/*");

    if let Some(ref r) = referer {
        builder = builder.header("referer", r);
    }

    if let Some(ref key) = api_key {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            builder = builder.header("X-API-Key", trimmed);
        }
    }

    let req = builder
        .body(wstd::io::empty())
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    let mut resp = Client::new()
        .send(req)
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    let status = resp.status();
    if status.as_u16() == 429 {
        return Err(ProviderError::RateLimited(None));
    }

    if !status.is_success() {
        let mut err_body = Vec::new();
        let _ = resp.body_mut().read_to_end(&mut err_body).await;
        let err_msg = String::from_utf8_lossy(&err_body);
        let trimmed_msg = err_msg.trim();

        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(ProviderError::Auth(if trimmed_msg.is_empty() {
                format!("HTTP {}", status)
            } else {
                format!("HTTP {}: {}", status, trimmed_msg)
            }));
        }

        return Err(ProviderError::Network(if trimmed_msg.is_empty() {
            format!("HTTP {}", status)
        } else {
            format!("HTTP {}: {}", status, trimmed_msg)
        }));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let mut body = Vec::new();
    resp.body_mut()
        .read_to_end(&mut body)
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    Ok((body, content_type))
}

struct WallhavenProvider;

impl Guest for WallhavenProvider {
    fn info() -> ProviderInfo {
        ProviderInfo {
            name: "wallhaven".to_string(),
            label: "Wallhaven".to_string(),
            version: "0.1.0".to_string(),
            options: vec![
                OptionSpec {
                    key: "query".to_string(),
                    label: "Search Query".to_string(),
                    description: Some(
                        "Search query or tags (e.g. nature, mountains, +waterfall -water, -{digital art})"
                            .to_string(),
                    ),
                    ty: ScalarType::Text,
                    multiple: false,
                    required: false,
                    default: None,
                },
                OptionSpec {
                    key: "categories".to_string(),
                    label: "Categories".to_string(),
                    description: Some(
                        "Categories to include: general, anime, people".to_string(),
                    ),
                    ty: ScalarType::Choice(vec![
                        "general".to_string(),
                        "anime".to_string(),
                        "people".to_string(),
                    ]),
                    multiple: true,
                    required: false,
                    default: Some(ConfigValue::Many(vec![ScalarValue::Choice(
                        "general".to_string(),
                    )])),
                },
                OptionSpec {
                    key: "allow_sketchy".to_string(),
                    label: "Allow Sketchy".to_string(),
                    description: Some("Include sketchy-rated wallpapers".to_string()),
                    ty: ScalarType::Boolean,
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Boolean(false))),
                },
                OptionSpec {
                    key: "allow_nsfw".to_string(),
                    label: "Allow NSFW".to_string(),
                    description: Some(
                        "Include NSFW-rated wallpapers (requires api_key)".to_string(),
                    ),
                    ty: ScalarType::Boolean,
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Boolean(false))),
                },
                OptionSpec {
                    key: "sorting".to_string(),
                    label: "Sorting".to_string(),
                    description: Some(
                        "Sorting method (random, favorites, toplist, views, relevance, date_added)"
                            .to_string(),
                    ),
                    ty: ScalarType::Choice(vec![
                        "random".to_string(),
                        "favorites".to_string(),
                        "toplist".to_string(),
                        "views".to_string(),
                        "relevance".to_string(),
                        "date_added".to_string(),
                    ]),
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Choice("random".to_string()))),
                },
                OptionSpec {
                    key: "top_range".to_string(),
                    label: "Toplist Time Range".to_string(),
                    description: Some(
                        "Time range when sorting is 'toplist' (1d, 3d, 1w, 1M, 3M, 6M, 1y)"
                            .to_string(),
                    ),
                    ty: ScalarType::Choice(vec![
                        "1d".to_string(),
                        "3d".to_string(),
                        "1w".to_string(),
                        "1M".to_string(),
                        "3M".to_string(),
                        "6M".to_string(),
                        "1y".to_string(),
                    ]),
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Choice("1M".to_string()))),
                },
                OptionSpec {
                    key: "api_key".to_string(),
                    label: "API Key".to_string(),
                    description: Some(
                        "Wallhaven API key (optional, required if allow_nsfw is enabled)"
                            .to_string(),
                    ),
                    ty: ScalarType::Secret,
                    multiple: false,
                    required: false,
                    default: None,
                },
            ],
            default_config: vec![
                ConfigEntry {
                    key: "categories".to_string(),
                    value: ConfigValue::Many(vec![ScalarValue::Choice("general".to_string())]),
                },
                ConfigEntry {
                    key: "allow_sketchy".to_string(),
                    value: ConfigValue::One(ScalarValue::Boolean(false)),
                },
                ConfigEntry {
                    key: "allow_nsfw".to_string(),
                    value: ConfigValue::One(ScalarValue::Boolean(false)),
                },
                ConfigEntry {
                    key: "sorting".to_string(),
                    value: ConfigValue::One(ScalarValue::Choice("random".to_string())),
                },
            ],
            allowed_hosts: vec![
                "wallhaven.cc".to_string(),
                "*.wallhaven.cc".to_string(),
                "w.wallhaven.cc".to_string(),
                "th.wallhaven.cc".to_string(),
            ],
        }
    }

    fn list(
        cfg: Config,
        limit: u32,
        _cursor: Option<String>,
        filter: FilterCriteria,
    ) -> Result<Page, ProviderError> {
        let reader = ConfigReader::new(&cfg);

        let allow_sketchy = reader.get_bool("allow_sketchy").unwrap_or(false);
        let allow_nsfw = reader.get_bool("allow_nsfw").unwrap_or(false);
        let purity = format!(
            "1{}{}",
            if allow_sketchy { "1" } else { "0" },
            if allow_nsfw { "1" } else { "0" }
        );

        let sorting = reader.get_choice("sorting").unwrap_or("random");
        let top_range = reader.get_choice("top_range").unwrap_or("1M");

        let orientation_param = match filter.orientation {
            Some(Orientation::Landscape) => Some("landscape"),
            Some(Orientation::Portrait) => Some("portrait"),
            Some(Orientation::Square) | Some(Orientation::Any) | None => None,
        };

        let mut api_url = Url::parse("https://wallhaven.cc/api/v1/search")
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        {
            let mut query = api_url.query_pairs_mut();

            let cats = reader.get_text_list("categories");
            let g = if cats.contains(&"general") || cats.is_empty() {
                '1'
            } else {
                '0'
            };
            let a = if cats.contains(&"anime") { '1' } else { '0' };
            let p = if cats.contains(&"people") { '1' } else { '0' };
            query.append_pair("categories", &format!("{}{}{}", g, a, p));
            query.append_pair("purity", &purity);
            query.append_pair("sorting", sorting);

            if sorting == "toplist" {
                query.append_pair("topRange", top_range);
            }

            if let Some(orient) = orientation_param {
                query.append_pair("ratios", orient);
            }

            if let Some(q) = reader.get_text("query") {
                let q_trimmed = q.trim();
                if !q_trimmed.is_empty() {
                    query.append_pair("q", q_trimmed);
                }
            }

            let api_key = reader
                .get_text("api_key")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            if let Some(key) = api_key {
                query.append_pair("apikey", key);
            }

            if let (Some(w), Some(h)) = (filter.min_width, filter.min_height) {
                query.append_pair("atleast", &format!("{}x{}", w, h));
            }
        }

        let api_key = reader
            .get_text("api_key")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let (body, _) =
            block_on(async move { http_get(api_url.as_str().to_string(), None, api_key).await })?;

        let parsed: WallhavenSearchResponse = serde_json::from_slice(&body).map_err(|e| {
            ProviderError::Other(format!("Failed to parse Wallhaven response: {}", e))
        })?;

        let mut items = Vec::new();
        for item in parsed.data {
            if let Some(min_w) = filter.min_width {
                if item.dimension_x < min_w {
                    continue;
                }
            }
            if let Some(min_h) = filter.min_height {
                if item.dimension_y < min_h {
                    continue;
                }
            }

            items.push(Wallpaper {
                id: item.path,
                title: Some(format!("Wallhaven {}", item.id)),
                author: item.source,
                source_url: Some(item.url),
                width: Some(item.dimension_x),
                height: Some(item.dimension_y),
            });

            if limit > 0 && items.len() >= limit as usize {
                break;
            }
        }

        Ok(Page {
            items,
            next_cursor: None,
        })
    }

    fn download(cfg: Config, id: String) -> Result<Image, ProviderError> {
        let reader = ConfigReader::new(&cfg);
        let api_key = reader
            .get_text("api_key")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let (data, ct) = block_on(async move {
            http_get(id, Some("https://wallhaven.cc/".to_string()), api_key).await
        })?;
        Ok(Image {
            content_type: ct.unwrap_or_else(|| "image/jpeg".to_string()),
            data,
        })
    }

    fn validate_config(cfg: Config) -> Result<Config, ProviderError> {
        Ok(cfg)
    }
}

wallpp_provider_sdk::export!(WallhavenProvider with_types_in wallpp_provider_sdk);
