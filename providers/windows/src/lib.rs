use serde::Deserialize;
use std::collections::HashSet;
use wallpp_provider_sdk::*;
use wstd::runtime::block_on;

const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";

#[derive(Deserialize)]
struct BatchResponse {
    batchrsp: Option<BatchRsp>,
}

#[derive(Deserialize)]
struct BatchRsp {
    #[serde(default)]
    items: Vec<BatchItemWrapper>,
}

#[derive(Deserialize)]
struct BatchItemWrapper {
    item: Option<String>,
}

// Response structure inside item string.
#[derive(Deserialize)]
struct SpotlightItem {
    ad: Option<SpotlightAd>,
}

#[derive(Deserialize)]
struct SpotlightAd {
    title: Option<String>,
    #[serde(rename = "iconHoverText")]
    icon_hover_text: Option<String>,
    copyright: Option<String>,
    #[serde(rename = "ctaUri")]
    cta_uri: Option<String>,
    #[serde(rename = "landscapeImage")]
    landscape_image: Option<SpotlightAsset>,
    #[serde(rename = "portraitImage")]
    portrait_image: Option<SpotlightAsset>,
}

#[derive(Deserialize)]
struct SpotlightAsset {
    asset: Option<String>,
}

fn parse_dimensions(url: &str) -> Option<(u32, u32)> {
    for part in url.split(|c: char| !c.is_ascii_alphanumeric()) {
        if let Some((w_str, h_str)) = part.split_once('x') {
            if let (Ok(w), Ok(h)) = (w_str.parse::<u32>(), h_str.parse::<u32>()) {
                if (100..=20000).contains(&w) && (100..=20000).contains(&h) {
                    return Some((w, h));
                }
            }
        }
    }
    None
}

async fn fetch(url: &str) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    http_get(
        url,
        &[("user-agent", USER_AGENT), ("accept", "*/*")],
        |_| false,
    )
    .await
}

struct WindowsProvider;

impl Guest for WindowsProvider {
    fn info() -> ProviderInfo {
        provider_info(
            "windows",
            "Windows Spotlight",
            "0.1.0",
            vec![
                spec("locale", "Locale", ScalarType::Text)
                    .desc("Language and regional locale (e.g. en-US, de-DE, ja-JP)")
                    .default_text("en-US")
                    .build(),
                spec("country", "Country", ScalarType::Text)
                    .desc("Two-letter ISO country code (e.g. US, DE, JP). Defaults to country from locale.")
                    .build(),
            ],
            &["locale"],
            &[
                "fd.api.iris.microsoft.com",
                "*.api.iris.microsoft.com",
                "*.iris.microsoft.com",
                "*.msn.com",
                "res.public.onecdn.static.microsoft",
                "*.onecdn.static.microsoft",
                "*.microsoft.com",
                "*.bing.com",
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
        let locale = reader.get_text("locale").unwrap_or("en-US");
        let country_buf: String;
        let country = if let Some(c) = reader.get_text("country") {
            c
        } else if let Some((_, region)) = locale.split_once('-') {
            country_buf = region.to_uppercase();
            &country_buf
        } else {
            "US"
        };

        let (want_landscape, want_portrait) = match filter.orientation {
            Some(Orientation::Landscape) => (true, false),
            Some(Orientation::Portrait) => (false, true),
            Some(Orientation::Square) => (false, false),
            Some(Orientation::Any) | None => (true, true),
        };

        let request_url = format!(
            "https://fd.api.iris.microsoft.com/v4/api/selection?&placement=88000820&bcnt=4&country={}&locale={}&fmt=json",
            country, locale
        );

        let (body, _) = block_on(async move { fetch(&request_url).await })?;
        let resp: BatchResponse =
            serde_json::from_slice(&body).map_err(|e| ProviderError::Network(e.to_string()))?;

        let items = resp.batchrsp.map(|r| r.items).unwrap_or_default();

        let mut wallpapers = Vec::new();
        let mut seen_ids = HashSet::new();

        for item_wrapper in items {
            let Some(item_str) = item_wrapper.item else {
                continue;
            };

            let Ok(item) = serde_json::from_str::<SpotlightItem>(&item_str) else {
                continue;
            };
            let Some(ad) = item.ad else {
                continue;
            };

            let title = ad.title.filter(|s| !s.trim().is_empty()).or_else(|| {
                ad.icon_hover_text.as_ref().and_then(|ht| {
                    ht.lines()
                        .next()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
            });

            let author = ad.copyright.filter(|s| !s.trim().is_empty()).or_else(|| {
                ad.icon_hover_text.as_ref().and_then(|ht| {
                    let mut lines = ht.lines();
                    let _ = lines.next();
                    lines
                        .next()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
            });

            let source_url = ad.cta_uri.as_ref().map(|uri| {
                if let Some(stripped) = uri.strip_prefix("microsoft-edge:") {
                    stripped.to_string()
                } else {
                    uri.clone()
                }
            });

            for (asset, want, fallback) in [
                (ad.landscape_image, want_landscape, (3840, 2160)),
                (ad.portrait_image, want_portrait, (1080, 1920)),
            ] {
                if !want {
                    continue;
                }
                let Some(asset_url) = asset
                    .and_then(|img| img.asset)
                    .filter(|u| u.starts_with("https://") && !u.ends_with("empty.jpg"))
                else {
                    continue;
                };
                if !seen_ids.insert(asset_url.clone()) {
                    continue;
                }
                let (w, h) = parse_dimensions(&asset_url).unwrap_or(fallback);
                wallpapers.push(Wallpaper {
                    id: asset_url,
                    title: title.clone(),
                    author: author.clone(),
                    source_url: source_url.clone(),
                    width: Some(w),
                    height: Some(h),
                });
                if limit > 0 && wallpapers.len() >= limit as usize {
                    break;
                }
            }
            if limit > 0 && wallpapers.len() >= limit as usize {
                break;
            }
        }

        Ok(Page {
            items: wallpapers,
            next_cursor: None,
        })
    }

    fn download(_cfg: Config, id: String) -> Result<Image, ProviderError> {
        let (data, ct) = block_on(async move { fetch(&id).await })?;
        Ok(Image {
            content_type: ct.unwrap_or_else(|| "image/jpeg".to_string()),
            data,
        })
    }

    fn validate_config(cfg: Config) -> Result<Config, ProviderError> {
        Ok(cfg)
    }
}

wallpp_provider_sdk::export!(WindowsProvider with_types_in wallpp_provider_sdk);
