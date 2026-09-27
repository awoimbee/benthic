//! Guards the static SEO surface of the web build.
//!
//! Search engines and social scrapers do not run the wasm app, so the
//! crawlable metadata has to live in files that are served verbatim:
//! the custom `index.html` shell, plus `robots.txt` and `sitemap.xml`
//! copied from `public/`.

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn index_html_has_core_seo_metadata() {
    let html = fs::read_to_string(root().join("index.html")).expect("read index.html");

    for needle in [
        "<title>",
        "name=\"description\"",
        "rel=\"canonical\"",
        "property=\"og:title\"",
        "property=\"og:description\"",
        "name=\"twitter:card\"",
        "name=\"google-site-verification\"",
        "application/ld+json",
        "id=\"main\"",
        "lang=\"en\"",
    ] {
        assert!(html.contains(needle), "index.html is missing `{needle}`");
    }
}

#[test]
fn robots_allows_crawling_and_points_at_sitemap() {
    let robots = fs::read_to_string(root().join("public/robots.txt")).expect("read robots.txt");
    assert!(robots.contains("User-agent: *"));
    assert!(robots.contains("Allow: /"));
    assert!(robots.contains("Sitemap: https://awoimbee.github.io/benthic/sitemap.xml"));
}

#[test]
fn sitemap_lists_the_app_url() {
    let sitemap = fs::read_to_string(root().join("public/sitemap.xml")).expect("read sitemap.xml");
    assert!(sitemap.contains("<urlset"));
    assert!(sitemap.contains("<loc>https://awoimbee.github.io/benthic/</loc>"));
}
