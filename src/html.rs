//! Small, allocation-light HTML scanning for Teams message bodies and link-preview metadata.
//! Rendering is done by GPUI Kit's `TextView::html`; this only rewrites Teams-specific tags
//! and extracts URLs. ponytail: a tag scanner, not a parser; malformed markup degrades to text.
use crate::model::Preview;

/// Every `<...>` tag in `html`, in order.
fn tags(html: &str) -> impl Iterator<Item = &str> {
    let mut rest = html;
    std::iter::from_fn(move || {
        let start = rest.find('<')?;
        let end = rest[start..]
            .find('>')
            .map_or(rest.len(), |end| start + end + 1);
        let tag = &rest[start..end];
        rest = &rest[end..];
        Some(tag)
    })
}

/// Lowercase tag name, with a leading `/` for closing tags.
fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('<')
        .split(|c: char| {
            c.is_ascii_whitespace() || c == '>' || (c == '/' && !tag.starts_with("</"))
        })
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The raw (still entity-encoded) value of attribute `name` in `tag`.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        if !lower[..at].ends_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let after = lower[from..].trim_start();
        if !after.starts_with('=') {
            continue;
        }
        let value = tag[tag.len() - after.len() + 1..].trim_start();
        return match value.chars().next()? {
            quote @ ('"' | '\'') => value[1..].find(quote).map(|end| &value[1..1 + end]),
            _ => value
                .split(|c: char| c.is_ascii_whitespace() || c == '>')
                .next(),
        };
    }
    None
}

pub(crate) fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn https(url: String) -> Option<String> {
    reqwest::Url::parse(&url)
        .is_ok_and(|url| url.scheme() == "https")
        .then_some(url)
}

/// Rewrites Teams-only tags into HTML that `TextView` understands: `<emoji>` becomes its
/// character, `<at>` mentions become bold, and inline file placeholders are dropped.
pub(crate) fn teams_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = rest.find('>').map_or(rest.len(), |end| end + 1);
        let tag = &rest[..end];
        match tag_name(tag).as_str() {
            "emoji" => {
                out.push_str(attr(tag, "alt").unwrap_or_default());
                if !tag.ends_with("/>")
                    && let Some(close) = rest.to_ascii_lowercase().find("</emoji>")
                {
                    rest = &rest[close + "</emoji>".len()..];
                    continue;
                }
            }
            "at" => out.push_str("<b>@"),
            "/at" => out.push_str("</b>"),
            "attachment" | "/attachment" | "systemeventmessage" | "/systemeventmessage" => {}
            _ => out.push_str(tag),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Whether `html` contains block content (lists, tables, quotes, code blocks).
pub(crate) fn has_blocks(html: &str) -> bool {
    tags(html).any(|tag| {
        matches!(
            tag_name(tag).as_str(),
            "ul" | "ol" | "table" | "blockquote" | "pre"
        )
    })
}

/// The https image sources (entity-decoded, as `TextView` resolves them) and the first https link.
pub(crate) fn media(html: &str) -> (Vec<String>, Option<String>) {
    let mut images = Vec::new();
    let mut link = None;
    for tag in tags(html) {
        match tag_name(tag).as_str() {
            "img" => images.extend(attr(tag, "src").map(unescape).and_then(https)),
            "a" if link.is_none() => link = attr(tag, "href").map(unescape).and_then(https),
            _ => {}
        }
    }
    (images, link)
}

/// Open Graph / Twitter / `<title>` metadata of a fetched page.
pub(crate) fn page_preview(page: &str, url: &reqwest::Url) -> Option<Preview> {
    let (mut title, mut description, mut image, mut site) = (None, None, None, None);
    for tag in tags(page) {
        if tag_name(tag) != "meta" {
            continue;
        }
        let Some(content) = attr(tag, "content").map(unescape) else {
            continue;
        };
        let key = attr(tag, "property")
            .or_else(|| attr(tag, "name"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let slot = match key.as_str() {
            "og:title" | "twitter:title" => &mut title,
            "og:description" | "twitter:description" | "description" => &mut description,
            "og:image" | "og:image:url" | "twitter:image" => &mut image,
            "og:site_name" => &mut site,
            _ => continue,
        };
        if slot.is_none() && !content.trim().is_empty() {
            *slot = Some(content.trim().to_owned());
        }
    }
    if title.is_none() {
        let lower = page.to_ascii_lowercase();
        let start = lower.find("<title")?;
        let start = start + lower[start..].find('>')? + 1;
        let end = start + lower[start..].find("</title")?;
        title = Some(unescape(page[start..end].trim())).filter(|title| !title.is_empty());
    }
    Some(Preview {
        url: url.to_string(),
        title: title?,
        description: description.unwrap_or_default(),
        image: image
            .and_then(|image| url.join(&image).ok())
            .map(String::from)
            .and_then(https),
        site: site.unwrap_or_else(|| url.host_str().unwrap_or_default().to_owned()),
    })
}
