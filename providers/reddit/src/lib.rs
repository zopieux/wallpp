use serde::Deserialize;
use url::Url;
use wallpp_provider_sdk::*;
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
struct PostData {
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

async fn fetch(
    url: &str,
    referer: Option<&str>,
) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    let mut headers = vec![("user-agent", USER_AGENT), ("accept", "*/*")];
    if let Some(referer) = referer {
        headers.push(("referer", referer));
    }
    http_get(url, &headers, |_| false).await
}

/// Validate a single-choice config entry against its allowed values.
fn check_choice(entry: &ConfigEntry, allowed: &[&str], label: &str) -> Result<(), ProviderError> {
    if let ConfigValue::One(ScalarValue::Choice(s)) = &entry.value {
        if !allowed.contains(&s.as_str()) {
            return Err(ProviderError::InvalidConfig(format!(
                "Invalid {} '{}'. Expected one of: {}.",
                label,
                s,
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

struct RedditProvider;

impl Guest for RedditProvider {
    fn info() -> ProviderInfo {
        provider_info(
            "reddit",
            "Reddit",
            "0.1.0",
            vec![
                spec("subreddits", "Subreddits", ScalarType::Text)
                    .desc("One subreddit per line.")
                    .multiple()
                    .required()
                    .default_texts(&["wallpapers"])
                    .build(),
                spec(
                    "sort",
                    "Sort method",
                    ScalarType::Choice(strings(&["hot", "new", "top"])),
                )
                .desc("Sorting method")
                .default_choice("top")
                .build(),
                spec(
                    "time",
                    "Top time window",
                    ScalarType::Choice(strings(&["hour", "day", "week", "month", "year", "all"])),
                )
                .desc("Time window when sort is 'top'")
                .default_choice("week")
                .build(),
            ],
            &["subreddits", "sort", "time"],
            &[
                "reddtastic.com",
                "www.reddtastic.com",
                "*.reddit.com",
                "reddit.com",
                "i.redd.it",
                "preview.redd.it",
                "external-preview.redd.it",
            ],
        )
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

        let mut api_url = Url::parse("https://reddtastic.com/api/reddit/subreddits/posts")
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        {
            let mut query = api_url.query_pairs_mut();
            query.append_pair("v", "3");
            query.append_pair("subreddits", &subreddits);
            query.append_pair("sort", sort);
            query.append_pair("limit", "100");

            if sort == "top" {
                query.append_pair("t", time);
            }

            if let Some(ref after) = cursor {
                query.append_pair("after", after);
            }
        }

        let first_sub = subs.first().copied().unwrap_or("wallpapers");
        let referer = Url::parse("https://reddtastic.com/r/")
            .ok()
            .and_then(|base| base.join(first_sub).ok())
            .map(|u| u.to_string());

        let (bytes, _) =
            block_on(async move { fetch(api_url.as_str(), referer.as_deref()).await })?;

        let res: ReddtasticResponse = serde_json::from_slice(&bytes)
            .map_err(|e| ProviderError::Other(format!("Failed to parse JSON: {}", e)))?;

        let data = res.data.ok_or(ProviderError::NotFound)?;
        let mut wallpapers = Vec::new();

        for child in data.children {
            let post = child.data;

            // Skip stickied announcements or pinned mod posts.
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

            // Skip external previews (thumbnails for text posts / news articles).
            if preview_img.url.contains("external-preview.redd.it") {
                continue;
            }

            // Determine image download URL: must be a real image link.
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
                _ => continue, // Not an image post.
            };

            let width = preview_img.width;
            let height = preview_img.height;
            let source_url = post.permalink.and_then(|p| {
                Url::parse("https://reddit.com")
                    .ok()
                    .and_then(|base| base.join(p.trim_start_matches('/')).ok())
                    .map(|u| u.to_string())
            });

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
        let (data, ct) = block_on(async move { fetch(&id, None).await })?;
        Ok(Image {
            content_type: ct.unwrap_or_else(|| "image/jpeg".to_string()),
            data,
        })
    }

    fn validate_config(cfg: Config) -> Result<Config, ProviderError> {
        let mut normalized = Vec::new();
        let mut has_subreddits = false;

        for entry in cfg {
            match entry.key.as_str() {
                "subreddits" => {
                    has_subreddits = true;
                    let raw_items: Vec<String> = match entry.value {
                        ConfigValue::One(ScalarValue::Text(s)) => vec![s],
                        ConfigValue::Many(items) => items
                            .into_iter()
                            .filter_map(|it| match it {
                                ScalarValue::Text(s) => Some(s),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    };

                    let mut subs: Vec<String> = raw_items
                        .iter()
                        .flat_map(|s| s.split(|c: char| c == ',' || c == '+' || c.is_whitespace()))
                        .filter_map(|s| {
                            let mut trimmed = s.trim();
                            if trimmed.is_empty() {
                                return None;
                            }
                            if let Some(rest) = trimmed.strip_prefix("https://") {
                                trimmed = rest;
                            } else if let Some(rest) = trimmed.strip_prefix("http://") {
                                trimmed = rest;
                            }
                            if let Some(idx) = trimmed.find("reddit.com/") {
                                trimmed = &trimmed[idx + "reddit.com/".len()..];
                            } else if trimmed == "reddit.com" || trimmed.ends_with(".reddit.com") {
                                return None;
                            }
                            if let Some(rest) = trimmed.strip_prefix("/r/") {
                                trimmed = rest;
                            } else if let Some(rest) = trimmed.strip_prefix("r/") {
                                trimmed = rest;
                            }
                            let trimmed = trimmed.trim_matches('/');
                            let name = trimmed.split('/').next().unwrap_or("").trim();
                            let name = name.split('?').next().unwrap_or("").trim();
                            if name.is_empty() {
                                None
                            } else {
                                Some(name.to_string())
                            }
                        })
                        .collect();

                    // Sort and deduplicate for determinism.
                    subs.sort();
                    subs.dedup();

                    if subs.is_empty() {
                        return Err(ProviderError::InvalidConfig(
                            "At least one subreddit must be specified.".to_string(),
                        ));
                    }

                    normalized.push(ConfigEntry {
                        key: "subreddits".to_string(),
                        value: ConfigValue::Many(subs.into_iter().map(ScalarValue::Text).collect()),
                    });
                }
                "sort" => {
                    check_choice(&entry, &["hot", "new", "top"], "sort method")?;
                    normalized.push(entry);
                }
                "time" => {
                    check_choice(
                        &entry,
                        &["hour", "day", "week", "month", "year", "all"],
                        "time window",
                    )?;
                    normalized.push(entry);
                }
                _ => {
                    normalized.push(entry);
                }
            }
        }

        if !has_subreddits {
            return Err(ProviderError::InvalidConfig(
                "Missing required option: 'subreddits'.".to_string(),
            ));
        }

        Ok(normalized)
    }
}

wallpp_provider_sdk::export!(RedditProvider with_types_in wallpp_provider_sdk);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_config_normalization() {
        let input = vec![ConfigEntry {
            key: "subreddits".to_string(),
            value: ConfigValue::One(ScalarValue::Text(
                "https://www.reddit.com/r/wallpapers, /r/earthporn\thttps://reddit.com/r/spaceporn/ + r/art\n  r/art   ".to_string(),
            )),
        }];

        let result = RedditProvider::validate_config(input).expect("Validation should succeed.");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, "subreddits");

        if let ConfigValue::Many(ref items) = result[0].value {
            let names: Vec<&str> = items
                .iter()
                .filter_map(|it| match it {
                    ScalarValue::Text(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            // Should be stripped of url, r/, split, sorted, and deduplicated.
            assert_eq!(names, vec!["art", "earthporn", "spaceporn", "wallpapers"]);
        } else {
            panic!("Expected ConfigValue::Many.");
        }
    }

    #[test]
    fn test_validate_config_empty_fails() {
        let input = vec![ConfigEntry {
            key: "subreddits".to_string(),
            value: ConfigValue::One(ScalarValue::Text("   ".to_string())),
        }];

        let err = RedditProvider::validate_config(input).unwrap_err();
        match err {
            ProviderError::InvalidConfig(_) => {}
            _ => panic!("Expected InvalidConfig error."),
        }
    }
}
