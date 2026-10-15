use serde::Deserialize;
use wallpp_provider_sdk::*;
use wstd::http::{Client, Request};
use wstd::io::AsyncRead;
use wstd::runtime::block_on;

const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";

#[derive(Deserialize)]
struct ReddtasticResponse {
    data: Option<ReddtasticData>,
}

#[derive(Deserialize)]
struct ReddtasticData {
    after: Option<String>,
    #[serde(default)]
    children: Vec<ReddtasticChild>,
}

#[derive(Deserialize)]
struct ReddtasticChild {
    data: PostData,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct PostData {
    id: String,
    title: String,
    author: Option<String>,
    permalink: Option<String>,
    url: Option<String>,
    preview: Option<Preview>,
    #[serde(default)]
    stickied: Option<bool>,
    #[serde(default)]
    pinned: Option<bool>,
}

#[derive(Deserialize)]
struct Preview {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    images: Vec<PreviewImage>,
}

#[derive(Deserialize)]
struct PreviewImage {
    source: ImageSource,
}

#[derive(Deserialize)]
struct ImageSource {
    url: String,
    width: u32,
    height: u32,
}

async fn http_get(
    url: String,
    referer: Option<String>,
) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    let mut builder = Request::get(&url)
        .header("user-agent", USER_AGENT)
        .header("accept", "*/*");

    if let Some(ref ref_url) = referer {
        builder = builder.header("referer", ref_url);
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
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(ProviderError::Auth(format!("HTTP {}", status)));
    }
    if !status.is_success() {
        return Err(ProviderError::Network(format!("HTTP {}", status)));
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

struct RedditProvider;

impl Guest for RedditProvider {
    fn info() -> ProviderInfo {
        ProviderInfo {
            name: "reddit".to_string(),
            label: "Reddit".to_string(),
            version: "0.1.0".to_string(),
            options: vec![
                OptionSpec {
                    key: "subreddits".to_string(),
                    label: "Subreddits".to_string(),
                    description: Some("List of subreddits to fetch wallpapers from".to_string()),
                    ty: ScalarType::Text,
                    multiple: true,
                    required: true,
                    default: Some(ConfigValue::Many(vec![ScalarValue::Text(
                        "wallpapers".to_string(),
                    )])),
                },
                OptionSpec {
                    key: "sort".to_string(),
                    label: "Sort method".to_string(),
                    description: Some("Sorting method".to_string()),
                    ty: ScalarType::Choice(vec![
                        "hot".to_string(),
                        "new".to_string(),
                        "top".to_string(),
                    ]),
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Choice("top".to_string()))),
                },
                OptionSpec {
                    key: "time".to_string(),
                    label: "Top time window".to_string(),
                    description: Some("Time window when sort is 'top'".to_string()),
                    ty: ScalarType::Choice(vec![
                        "hour".to_string(),
                        "day".to_string(),
                        "week".to_string(),
                        "month".to_string(),
                        "year".to_string(),
                        "all".to_string(),
                    ]),
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Choice("week".to_string()))),
                },
            ],
            default_config: vec![
                ConfigEntry {
                    key: "subreddits".to_string(),
                    value: ConfigValue::Many(vec![ScalarValue::Text("wallpapers".to_string())]),
                },
                ConfigEntry {
                    key: "sort".to_string(),
                    value: ConfigValue::One(ScalarValue::Choice("top".to_string())),
                },
                ConfigEntry {
                    key: "time".to_string(),
                    value: ConfigValue::One(ScalarValue::Choice("week".to_string())),
                },
            ],
            allowed_hosts: vec![
                "reddtastic.com".to_string(),
                "www.reddtastic.com".to_string(),
                "*.reddit.com".to_string(),
                "reddit.com".to_string(),
                "i.redd.it".to_string(),
                "preview.redd.it".to_string(),
                "external-preview.redd.it".to_string(),
            ],
        }
    }

    fn list(
        cfg: Config,
        limit: u32,
        cursor: Option<String>,
        _filter: FilterCriteria,
    ) -> Result<Page, ProviderError> {
        let reader = ConfigReader::new(&cfg);
        let subs = reader.get_text_list("subreddits");
        let subreddits = if subs.is_empty() {
            "wallpapers".to_string()
        } else {
            subs.join("+")
        };

        let sort = reader.get_choice("sort").unwrap_or("hot");
        let time = reader.get_choice("time").unwrap_or("week");

        let mut url = format!(
            "https://reddtastic.com/api/reddit/subreddits/posts?v=3&subreddits={}&sort={}&limit=100",
            subreddits, sort
        );

        if sort == "top" {
            url.push_str(&format!("&t={}", time));
        }

        if let Some(ref after) = cursor {
            url.push_str(&format!("&after={}", after));
        }

        let first_sub = subs.first().copied().unwrap_or("wallpapers");
        let referer = Some(format!("https://reddtastic.com/r/{}", first_sub));

        let (bytes, _) = block_on(async move { http_get(url, referer).await })?;

        let res: ReddtasticResponse = serde_json::from_slice(&bytes)
            .map_err(|e| ProviderError::Other(format!("Failed to parse JSON: {}", e)))?;

        let data = res.data.ok_or(ProviderError::NotFound)?;
        let mut wallpapers = Vec::new();

        for child in data.children {
            let post = child.data;

            // Skip stickied announcements or pinned mod posts
            if post.stickied.unwrap_or(false) || post.pinned.unwrap_or(false) {
                continue;
            }

            let preview = match post.preview {
                Some(p) if p.enabled != Some(false) => p,
                _ => continue,
            };

            let preview_img = match preview.images.into_iter().next() {
                Some(img) => img.source,
                None => continue,
            };

            // Skip external previews (thumbnails for text posts / news articles)
            if preview_img.url.contains("external-preview.redd.it") {
                continue;
            }

            // Determine image download URL: must be a real image link
            let image_url = match post.url {
                Some(ref u)
                    if u.contains("i.redd.it")
                        || u.contains("i.imgur.com")
                        || u.ends_with(".jpg")
                        || u.ends_with(".png")
                        || u.ends_with(".jpeg")
                        || u.ends_with(".webp") =>
                {
                    u.clone()
                }
                _ if preview_img.url.contains("preview.redd.it") => {
                    preview_img.url.replace("&amp;", "&")
                }
                _ => continue, // Not an image post
            };

            let width = preview_img.width;
            let height = preview_img.height;
            let source_url = post.permalink.map(|p| format!("https://reddit.com{}", p));

            wallpapers.push(Wallpaper {
                id: image_url,
                title: Some(post.title),
                author: post.author,
                source_url,
                width: Some(width),
                height: Some(height),
            });

            if limit > 0 && wallpapers.len() >= limit as usize {
                break;
            }
        }

        Ok(Page {
            items: wallpapers,
            next_cursor: data.after,
        })
    }

    fn download(_cfg: Config, id: String) -> Result<Image, ProviderError> {
        let (data, ct) = block_on(async move { http_get(id, None).await })?;
        Ok(Image {
            content_type: ct.unwrap_or_else(|| "image/jpeg".to_string()),
            data,
        })
    }
}

wallpp_provider_sdk::export!(RedditProvider with_types_in wallpp_provider_sdk);
