use serde::Deserialize;
use std::collections::HashSet;
use wallpp_provider_sdk::*;
use wstd::http::{Client, Request};
use wstd::io::AsyncRead;
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

// v4 response structure inside item string.
#[derive(Deserialize)]
struct SpotlightItemV4 {
    ad: Option<SpotlightAdV4>,
}

#[derive(Deserialize)]
struct SpotlightAdV4 {
    title: Option<String>,
    #[serde(rename = "iconHoverText")]
    icon_hover_text: Option<String>,
    copyright: Option<String>,
    #[serde(rename = "ctaUri")]
    cta_uri: Option<String>,
    #[serde(rename = "landscapeImage")]
    landscape_image: Option<SpotlightAssetV4>,
    #[serde(rename = "portraitImage")]
    portrait_image: Option<SpotlightAssetV4>,
}

#[derive(Deserialize)]
struct SpotlightAssetV4 {
    asset: Option<String>,
}

// v3 response structure inside item string.
#[derive(Deserialize)]
struct SpotlightItemV3 {
    ad: Option<SpotlightAdV3>,
}

#[derive(Deserialize)]
struct SpotlightAdV3 {
    title_text: Option<TextPropertyV3>,
    copyright_text: Option<TextPropertyV3>,
    image_fullscreen_001_landscape: Option<ImageV3>,
    image_fullscreen_001_portrait: Option<ImageV3>,
}

#[derive(Deserialize)]
struct TextPropertyV3 {
    tx: Option<String>,
}

#[derive(Deserialize)]
struct ImageV3 {
    u: Option<String>,
    w: Option<String>,
    h: Option<String>,
    #[serde(rename = "fileSize")]
    file_size: Option<String>,
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

struct WindowsProvider;

impl Guest for WindowsProvider {
    fn info() -> ProviderInfo {
        ProviderInfo {
            name: "windows".to_string(),
            label: "Windows Spotlight".to_string(),
            version: "0.1.0".to_string(),
            options: vec![
                OptionSpec {
                    key: "locale".to_string(),
                    label: "Locale".to_string(),
                    description: Some(
                        "Language and regional locale (e.g. en-US, de-DE, ja-JP)".to_string(),
                    ),
                    ty: ScalarType::Text,
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Text("en-US".to_string()))),
                },
                OptionSpec {
                    key: "country".to_string(),
                    label: "Country".to_string(),
                    description: Some(
                        "Two-letter ISO country code (e.g. US, DE, JP). Defaults to country from locale."
                            .to_string(),
                    ),
                    ty: ScalarType::Text,
                    multiple: false,
                    required: false,
                    default: None,
                },
                OptionSpec {
                    key: "orientation".to_string(),
                    label: "Preferred Orientation".to_string(),
                    description: Some(
                        "Preferred wallpaper orientation when not filtered (landscape, portrait, both)"
                            .to_string(),
                    ),
                    ty: ScalarType::Choice(vec![
                        "landscape".to_string(),
                        "portrait".to_string(),
                        "both".to_string(),
                    ]),
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Choice("landscape".to_string()))),
                },
                OptionSpec {
                    key: "api_version".to_string(),
                    label: "API Version".to_string(),
                    description: Some(
                        "Spotlight API version: v4 (4K, Windows 11) or v3 (1080p, Windows 10)"
                            .to_string(),
                    ),
                    ty: ScalarType::Choice(vec!["v4".to_string(), "v3".to_string()]),
                    multiple: false,
                    required: false,
                    default: Some(ConfigValue::One(ScalarValue::Choice("v4".to_string()))),
                },
            ],
            default_config: vec![
                ConfigEntry {
                    key: "locale".to_string(),
                    value: ConfigValue::One(ScalarValue::Text("en-US".to_string())),
                },
                ConfigEntry {
                    key: "orientation".to_string(),
                    value: ConfigValue::One(ScalarValue::Choice("landscape".to_string())),
                },
                ConfigEntry {
                    key: "api_version".to_string(),
                    value: ConfigValue::One(ScalarValue::Choice("v4".to_string())),
                },
            ],
            allowed_hosts: vec![
                "fd.api.iris.microsoft.com".to_string(),
                "arc.msn.com".to_string(),
                "*.api.iris.microsoft.com".to_string(),
                "*.iris.microsoft.com".to_string(),
                "*.msn.com".to_string(),
                "res.public.onecdn.static.microsoft".to_string(),
                "*.onecdn.static.microsoft".to_string(),
                "*.microsoft.com".to_string(),
                "*.bing.com".to_string(),
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

        let pref_orientation = reader.get_choice("orientation").unwrap_or("landscape");
        let api_version = reader.get_choice("api_version").unwrap_or("v4");

        let want_landscape = match filter.orientation {
            Some(Orientation::Landscape) => true,
            Some(Orientation::Portrait) => false,
            Some(Orientation::Square) => false,
            Some(Orientation::Any) | None => match pref_orientation {
                "portrait" => false,
                "both" | "all" => true,
                _ => true,
            },
        };

        let want_portrait = match filter.orientation {
            Some(Orientation::Portrait) => true,
            Some(Orientation::Landscape) => false,
            Some(Orientation::Square) => false,
            Some(Orientation::Any) | None => {
                matches!(pref_orientation, "portrait" | "both" | "all")
            }
        };

        let request_url = if api_version == "v3" {
            format!(
                "https://arc.msn.com/v3/Delivery/Placement?pid=209567&fmt=json&rafb=0&ua=WindowsShellClient%2F0&cdm=1&disphorzres=9999&dispvertres=9999&lo=80217&pl={}&lc={}&ctry={}&time=2026-12-31T23:59:59Z",
                locale,
                locale,
                country.to_lowercase()
            )
        } else {
            format!(
                "https://fd.api.iris.microsoft.com/v4/api/selection?&placement=88000820&bcnt=4&country={}&locale={}&fmt=json",
                country, locale
            )
        };

        let (body, _) = block_on(async move { http_get(request_url).await })?;
        let resp: BatchResponse =
            serde_json::from_slice(&body).map_err(|e| ProviderError::Network(e.to_string()))?;

        let items = resp.batchrsp.map(|r| r.items).unwrap_or_default();

        let mut wallpapers = Vec::new();
        let mut seen_ids = HashSet::new();

        for item_wrapper in items {
            let Some(item_str) = item_wrapper.item else {
                continue;
            };

            if api_version == "v3" {
                let Ok(item_v3) = serde_json::from_str::<SpotlightItemV3>(&item_str) else {
                    continue;
                };
                let Some(ad) = item_v3.ad else {
                    continue;
                };

                let title = ad
                    .title_text
                    .and_then(|t| t.tx)
                    .filter(|s| !s.trim().is_empty());
                let author = ad
                    .copyright_text
                    .and_then(|t| t.tx)
                    .filter(|s| !s.trim().is_empty());

                if want_landscape {
                    if let Some(img) = ad.image_fullscreen_001_landscape {
                        if let Some(u) = img.u {
                            let size = img
                                .file_size
                                .and_then(|s| s.parse::<u64>().ok())
                                .unwrap_or(0);
                            if !u.ends_with("empty.jpg") && size > 736 && seen_ids.insert(u.clone())
                            {
                                let w = img.w.and_then(|s| s.parse::<u32>().ok()).or(Some(1920));
                                let h = img.h.and_then(|s| s.parse::<u32>().ok()).or(Some(1080));
                                wallpapers.push(Wallpaper {
                                    id: u,
                                    title: title.clone(),
                                    author: author.clone(),
                                    source_url: None,
                                    width: w,
                                    height: h,
                                });
                                if limit > 0 && wallpapers.len() >= limit as usize {
                                    break;
                                }
                            }
                        }
                    }
                }

                if want_portrait {
                    if let Some(img) = ad.image_fullscreen_001_portrait {
                        if let Some(u) = img.u {
                            let size = img
                                .file_size
                                .and_then(|s| s.parse::<u64>().ok())
                                .unwrap_or(0);
                            if !u.ends_with("empty.jpg") && size > 736 && seen_ids.insert(u.clone())
                            {
                                let w = img.w.and_then(|s| s.parse::<u32>().ok()).or(Some(1080));
                                let h = img.h.and_then(|s| s.parse::<u32>().ok()).or(Some(1920));
                                wallpapers.push(Wallpaper {
                                    id: u,
                                    title: title.clone(),
                                    author: author.clone(),
                                    source_url: None,
                                    width: w,
                                    height: h,
                                });
                                if limit > 0 && wallpapers.len() >= limit as usize {
                                    break;
                                }
                            }
                        }
                    }
                }
            } else {
                let Ok(item_v4) = serde_json::from_str::<SpotlightItemV4>(&item_str) else {
                    continue;
                };
                let Some(ad) = item_v4.ad else {
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

                if want_landscape {
                    if let Some(asset_url) = ad
                        .landscape_image
                        .and_then(|img| img.asset)
                        .filter(|u| u.starts_with("https://") && !u.ends_with("empty.jpg"))
                    {
                        if seen_ids.insert(asset_url.clone()) {
                            let (w, h) = parse_dimensions(&asset_url).unwrap_or((3840, 2160));
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
                    }
                }

                if want_portrait {
                    if let Some(asset_url) = ad
                        .portrait_image
                        .and_then(|img| img.asset)
                        .filter(|u| u.starts_with("https://") && !u.ends_with("empty.jpg"))
                    {
                        if seen_ids.insert(asset_url.clone()) {
                            let (w, h) = parse_dimensions(&asset_url).unwrap_or((1080, 1920));
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
                    }
                }
            }
        }

        Ok(Page {
            items: wallpapers,
            next_cursor: None,
        })
    }

    fn download(_cfg: Config, id: String) -> Result<Image, ProviderError> {
        let (data, ct) = block_on(async move { http_get(id).await })?;
        Ok(Image {
            content_type: ct.unwrap_or_else(|| "image/jpeg".to_string()),
            data,
        })
    }
}

wallpp_provider_sdk::export!(WindowsProvider with_types_in wallpp_provider_sdk);
