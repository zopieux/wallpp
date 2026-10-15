use serde::Deserialize;
use url::Url;
use wallpp_provider_sdk::*;
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

async fn fetch(
    url: &str,
    referer: Option<&str>,
    api_key: Option<String>,
) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    let mut headers = vec![("user-agent", USER_AGENT), ("accept", "*/*")];
    if let Some(referer) = referer {
        headers.push(("referer", referer));
    }
    if let Some(key) = api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        headers.push(("X-API-Key", key));
    }
    http_get(url, &headers, |_| false).await
}

struct WallhavenProvider;

impl Guest for WallhavenProvider {
    fn info() -> ProviderInfo {
        provider_info(
            "wallhaven",
            "Wallhaven",
            "0.1.0",
            vec![
                spec("query", "Search Query", ScalarType::Text)
                    .desc("Search query or tags (e.g. nature, mountains, +waterfall -water, -{digital art})")
                    .build(),
                spec(
                    "categories",
                    "Categories",
                    ScalarType::Choice(strings(&["general", "anime", "people"])),
                )
                .desc("Categories to include: general, anime, people")
                .multiple()
                .default_choices(&["general"])
                .build(),
                spec("allow_sketchy", "Allow Sketchy", ScalarType::Boolean)
                    .desc("Include sketchy-rated wallpapers")
                    .default_bool(false)
                    .build(),
                spec("allow_nsfw", "Allow NSFW", ScalarType::Boolean)
                    .desc("Include NSFW-rated wallpapers (requires api_key)")
                    .default_bool(false)
                    .build(),
                spec(
                    "sorting",
                    "Sorting",
                    ScalarType::Choice(strings(&[
                        "random",
                        "favorites",
                        "toplist",
                        "views",
                        "relevance",
                        "date_added",
                    ])),
                )
                .desc("Sorting method (random, favorites, toplist, views, relevance, date_added)")
                .default_choice("random")
                .build(),
                spec(
                    "top_range",
                    "Toplist Time Range",
                    ScalarType::Choice(strings(&["1d", "3d", "1w", "1M", "3M", "6M", "1y"])),
                )
                .desc("Time range when sorting is 'toplist' (1d, 3d, 1w, 1M, 3M, 6M, 1y)")
                .default_choice("1M")
                .build(),
                spec("api_key", "API Key", ScalarType::Secret)
                    .desc("Wallhaven API key (optional, required if allow_nsfw is enabled)")
                    .build(),
            ],
            &["categories", "allow_sketchy", "allow_nsfw", "sorting"],
            &[
                "wallhaven.cc",
                "*.wallhaven.cc",
                "w.wallhaven.cc",
                "th.wallhaven.cc",
            ],
        )
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

        let (body, _) = block_on(async move { fetch(api_url.as_str(), None, api_key).await })?;

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

        let (data, ct) =
            block_on(async move { fetch(&id, Some("https://wallhaven.cc/"), api_key).await })?;
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
