use serde::{Deserialize, Serialize};
use url::Url;
use wallpp_provider_sdk::*;
use wstd::http::{Client, Request};
use wstd::io::AsyncRead;
use wstd::runtime::block_on;

const USER_AGENT: &str = "wallpp/0.1.0";
const DEFAULT_CLIENT_ID: &str = "072e5048dfcb73a8d9ad59fcf402471518ff8df725df462b0c4fa665f466515a";

#[derive(Deserialize)]
struct UnsplashPhoto {
    width: u32,
    height: u32,
    description: Option<String>,
    alt_description: Option<String>,
    urls: UnsplashUrls,
    links: Option<UnsplashLinks>,
    user: Option<UnsplashUser>,
}

#[derive(Deserialize)]
struct UnsplashUrls {
    full: String,
}

#[derive(Deserialize)]
struct UnsplashLinks {
    html: Option<String>,
    download_location: Option<String>,
}

#[derive(Deserialize)]
struct UnsplashUser {
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RandomPhotosResponse {
    List(Vec<UnsplashPhoto>),
    Single(UnsplashPhoto),
}

#[derive(Serialize, Deserialize)]
struct DownloadPayload {
    url: String,
    download_location: Option<String>,
}

async fn http_get(url: String) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    let req = Request::get(&url)
        .header("user-agent", USER_AGENT)
        .header("accept", "*/*")
        .body(wstd::io::empty())
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    let mut resp = Client::new()
        .send(req)
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    let status = resp.status();
    let ratelimit_remaining = resp
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    if status.as_u16() == 429 || (status.as_u16() == 403 && ratelimit_remaining == Some(0)) {
        return Err(ProviderError::RateLimited(None));
    }

    if !status.is_success() {
        let mut err_body = Vec::new();
        let _ = resp.body_mut().read_to_end(&mut err_body).await;
        let err_msg = String::from_utf8_lossy(&err_body);
        let trimmed_msg = err_msg.trim();

        if trimmed_msg.to_lowercase().contains("rate limit") {
            return Err(ProviderError::RateLimited(None));
        }

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

struct UnsplashProvider;

impl Guest for UnsplashProvider {
    fn info() -> ProviderInfo {
        ProviderInfo {
            name: "unsplash".to_string(),
            label: "Unsplash".to_string(),
            version: "0.1.0".to_string(),
            options: vec![
                OptionSpec {
                    key: "api_key".to_string(),
                    label: "API Key".to_string(),
                    description: Some("Unsplash Access Key / Client ID".to_string()),
                    ty: ScalarType::Secret,
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Text(
                        DEFAULT_CLIENT_ID.to_string(),
                    ))),
                },
                OptionSpec {
                    key: "orientation".to_string(),
                    label: "Orientation".to_string(),
                    description: Some(
                        "Preferred photo orientation (landscape, portrait, squarish, or any)"
                            .to_string(),
                    ),
                    ty: ScalarType::Choice(vec![
                        "landscape".to_string(),
                        "portrait".to_string(),
                        "squarish".to_string(),
                        "any".to_string(),
                    ]),
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Choice(
                        "landscape".to_string(),
                    ))),
                },
                OptionSpec {
                    key: "query".to_string(),
                    label: "Search Query".to_string(),
                    description: Some(
                        "Search terms or keywords (e.g. nature, wallpapers, architecture)"
                            .to_string(),
                    ),
                    ty: ScalarType::Text,
                    multiple: false,
                    required: false,
                    default: None,
                },
                OptionSpec {
                    key: "topics".to_string(),
                    label: "Topics".to_string(),
                    description: Some(
                        "Public topic IDs or slugs (e.g. wallpapers, nature, travel)".to_string(),
                    ),
                    ty: ScalarType::Text,
                    multiple: false,
                    required: false,
                    default: None,
                },
            ],
            default_config: vec![
                ConfigEntry {
                    key: "api_key".to_string(),
                    value: ConfigValue::One(ScalarValue::Text(DEFAULT_CLIENT_ID.to_string())),
                },
                ConfigEntry {
                    key: "orientation".to_string(),
                    value: ConfigValue::One(ScalarValue::Choice("landscape".to_string())),
                },
            ],
            allowed_hosts: vec![
                "api.unsplash.com".to_string(),
                "images.unsplash.com".to_string(),
                "*.unsplash.com".to_string(),
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
        let client_id = reader
            .get_text("api_key")
            .unwrap_or(DEFAULT_CLIENT_ID)
            .trim();
        if client_id.is_empty() {
            return Err(ProviderError::Auth(
                "Unsplash API key is missing".to_string(),
            ));
        }

        let orientation_choice = reader.get_choice("orientation").unwrap_or("landscape");
        let orientation_param = match filter.orientation {
            Some(Orientation::Landscape) => Some("landscape"),
            Some(Orientation::Portrait) => Some("portrait"),
            Some(Orientation::Square) => Some("squarish"),
            Some(Orientation::Any) | None => match orientation_choice {
                "landscape" => Some("landscape"),
                "portrait" => Some("portrait"),
                "squarish" => Some("squarish"),
                _ => None,
            },
        };

        let count = if limit > 0 { limit.min(30) } else { 30 };
        let mut api_url = Url::parse("https://api.unsplash.com/photos/random")
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        {
            let mut query = api_url.query_pairs_mut();
            query.append_pair("count", &count.to_string());
            query.append_pair("client_id", client_id);

            if let Some(orient) = orientation_param {
                query.append_pair("orientation", orient);
            }

            if let Some(q) = reader.get_text("query") {
                let q_trimmed = q.trim();
                if !q_trimmed.is_empty() {
                    query.append_pair("query", q_trimmed);
                }
            }

            if let Some(t) = reader.get_text("topics") {
                let t_trimmed = t.trim();
                if !t_trimmed.is_empty() {
                    query.append_pair("topics", t_trimmed);
                }
            }
        }

        let (body, _) = block_on(async move { http_get(api_url.as_str().to_string()).await })?;

        let parsed: RandomPhotosResponse = serde_json::from_slice(&body).map_err(|e| {
            ProviderError::Other(format!("Failed to parse Unsplash response: {}", e))
        })?;

        let photos = match parsed {
            RandomPhotosResponse::List(list) => list,
            RandomPhotosResponse::Single(photo) => vec![photo],
        };

        let mut items = Vec::new();
        for photo in photos {
            // Exclude watermarked/plus photos.
            if photo.urls.full.contains("plus.unsplash.com/") {
                continue;
            }

            if let Some(min_w) = filter.min_width {
                if photo.width < min_w {
                    continue;
                }
            }
            if let Some(min_h) = filter.min_height {
                if photo.height < min_h {
                    continue;
                }
            }

            let source_url = photo.links.as_ref().and_then(|l| l.html.as_ref()).map(|h| {
                if let Ok(mut u) = Url::parse(h) {
                    u.query_pairs_mut()
                        .append_pair("utm_source", "wallpp")
                        .append_pair("utm_medium", "referral");
                    u.to_string()
                } else {
                    h.clone()
                }
            });

            let download_location = photo
                .links
                .as_ref()
                .and_then(|l| l.download_location.clone());
            let payload = DownloadPayload {
                url: photo.urls.full,
                download_location,
            };
            let id = serde_json::to_string(&payload).unwrap_or(payload.url);

            let title = photo.description.or(photo.alt_description);
            let author = photo.user.and_then(|u| u.name);

            items.push(Wallpaper {
                id,
                title,
                author,
                source_url,
                width: Some(photo.width),
                height: Some(photo.height),
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
        let (target_url, download_location) =
            if let Ok(payload) = serde_json::from_str::<DownloadPayload>(&id) {
                (payload.url, payload.download_location)
            } else {
                (id, None)
            };

        // Report download to Unsplash per API terms.
        if let Some(dl_url) = download_location {
            let reader = ConfigReader::new(&cfg);
            let client_id = reader
                .get_text("api_key")
                .unwrap_or(DEFAULT_CLIENT_ID)
                .trim();
            if !client_id.is_empty() {
                if let Ok(mut ping_url) = Url::parse(&dl_url) {
                    ping_url
                        .query_pairs_mut()
                        .append_pair("client_id", client_id);
                    let ping_str = ping_url.to_string();
                    let _ = block_on(async move { http_get(ping_str).await });
                }
            }
        }

        let (data, ct) = block_on(async move { http_get(target_url).await })?;
        Ok(Image {
            content_type: ct.unwrap_or_else(|| "image/jpeg".to_string()),
            data,
        })
    }
}

wallpp_provider_sdk::export!(UnsplashProvider with_types_in wallpp_provider_sdk);
