//! External photo and album links attached to a dive.
//!
//! Subsurface stores pictures as filenames. We reuse that field for public
//! URLs so the links survive SSRF and Subsurface cloud round-trips. Image URLs
//! are rendered inline; Google Photos and iCloud shared albums get an embedded
//! viewer with an always-available external link because providers may refuse
//! to be framed.

use dioxus::prelude::*;

use benthic_core::Picture;

use crate::i18n;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaKind {
    Image,
    GooglePhotos,
    ICloud,
    Link,
}

/// Normalise a user-entered public URL. Returns `None` for non-URL filenames
/// so imported local pictures are left untouched.
pub fn normalize_media_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(trimmed.to_string());
    }
    if trimmed.contains(char::is_whitespace) {
        return None;
    }
    let host = trimmed.split('/').next().unwrap_or(trimmed);
    let looks_like_host = host.contains('.')
        && (trimmed.contains('/')
            || lower.starts_with("www.")
            || lower.starts_with("photos.")
            || lower.starts_with("share."));
    looks_like_host.then(|| format!("https://{trimmed}"))
}

fn host_and_path(url: &str) -> Option<(&str, &str)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let rest = &rest[..end];
    let (authority, path) = rest.split_once('/').unwrap_or((rest, "/"));
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then_some((host, path))
}

fn is_image_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "bmp" | "svg" | "heic" | "heif"
    )
}

fn is_image_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "googleusercontent.com"
        || host.ends_with(".googleusercontent.com")
        || host.ends_with(".ggpht.com")
        || host.ends_with(".icloud-content.com")
}

fn classify(url: &str) -> MediaKind {
    let Some((host, path)) = host_and_path(url) else {
        return MediaKind::Link;
    };
    let host = host.to_ascii_lowercase();
    if host == "photos.app.goo.gl" || host == "photos.google.com" {
        MediaKind::GooglePhotos
    } else if ((host == "www.icloud.com" || host == "icloud.com")
        && path.trim_start_matches('/').starts_with("sharedalbum"))
        || (host == "share.icloud.com" && path.trim_start_matches('/').starts_with("photos"))
    {
        MediaKind::ICloud
    } else if is_image_path(path) || is_image_host(&host) {
        MediaKind::Image
    } else {
        MediaKind::Link
    }
}

#[component]
pub fn MediaGallery(pictures: Vec<Picture>) -> Element {
    let state = use_context::<crate::state::AppState>();
    let tr = i18n::strings((state.prefs)().language);
    let items: Vec<(String, MediaKind)> = pictures
        .iter()
        .filter_map(|picture| {
            let url = normalize_media_url(&picture.filename)?;
            let kind = classify(&url);
            Some((url, kind))
        })
        .collect();
    if items.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "section-title", "{tr.section_media}" }
        div { class: "media-grid",
            for (index, (url, kind)) in items.into_iter().enumerate() {
                {media_item(index, url, kind, tr)}
            }
        }
    }
}

fn media_item(index: usize, url: String, kind: MediaKind, tr: &'static i18n::Strings) -> Element {
    let key = format!("media-{index}");
    match kind {
        MediaKind::Image => rsx! {
            a {
                key: "{key}",
                class: "media-item media-image",
                href: "{url}",
                target: "_blank",
                rel: "noopener noreferrer",
                img {
                    class: "media-thumb",
                    src: "{url}",
                    alt: "{tr.media_photo}",
                    loading: "lazy",
                }
            }
        },
        MediaKind::GooglePhotos => album_card(key, url, tr.media_google_photos, tr),
        MediaKind::ICloud => album_card(key, url, tr.media_icloud, tr),
        MediaKind::Link => rsx! {
            a {
                key: "{key}",
                class: "media-item media-link",
                href: "{url}",
                target: "_blank",
                rel: "noopener noreferrer",
                span { class: "media-provider", "{tr.media_link}" }
                span { class: "media-url", "{url}" }
                span { class: "media-open", "{tr.open} ↗" }
            }
        },
    }
}

fn album_card(
    key: String,
    url: String,
    provider: &'static str,
    tr: &'static i18n::Strings,
) -> Element {
    rsx! {
        div { key: "{key}", class: "media-item media-album",
            div { class: "media-album-head",
                span { class: "media-provider", "{provider}" }
                a {
                    class: "media-open",
                    href: "{url}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "{tr.open} ↗"
                }
            }
            div { class: "media-embed",
                iframe {
                    src: "{url}",
                    title: "{provider}",
                    allowfullscreen: "true",
                }
            }
            p { class: "media-hint", "{tr.media_embed_hint}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_provider_links() {
        assert_eq!(
            classify("https://photos.app.goo.gl/abc123"),
            MediaKind::GooglePhotos
        );
        assert_eq!(
            classify("https://photos.google.com/share/abc?key=def"),
            MediaKind::GooglePhotos
        );
        assert_eq!(
            classify("https://www.icloud.com/sharedalbum/#B0abc"),
            MediaKind::ICloud
        );
    }

    #[test]
    fn recognises_direct_images() {
        assert_eq!(
            classify("https://example.com/dive.jpg?size=large"),
            MediaKind::Image
        );
        assert_eq!(
            classify("https://lh3.googleusercontent.com/abc=w1200"),
            MediaKind::Image
        );
        assert_eq!(classify("https://example.com/gallery"), MediaKind::Link);
    }

    #[test]
    fn normalises_short_links_without_changing_local_paths() {
        assert_eq!(
            normalize_media_url("photos.app.goo.gl/abc123").as_deref(),
            Some("https://photos.app.goo.gl/abc123")
        );
        assert_eq!(normalize_media_url("Pictures/dive.jpg"), None);
        assert_eq!(normalize_media_url("IMG_1234.JPG"), None);
    }
}
