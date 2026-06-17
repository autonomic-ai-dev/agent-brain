//! Fetch documentation pages and strip HTML to plain text.

use anyhow::{bail, Context, Result};
use regex::Regex;

pub fn fetch_url(url: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let max_bytes = max_bytes.max(4096).min(5_000_000);
    let response = ureq::get(url)
        .header("User-Agent", "agent-brain/0.18 (+https://github.com/aeswibon/agent-brain)")
        .header("Accept", "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.8")
        .call()
        .with_context(|| format!("GET {url}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        bail!("HTTP {status} for {url}");
    }
    let mut body = Vec::new();
    let mut reader = response.into_body().into_reader();
    let mut buf = [0u8; 8192];
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf)
            .with_context(|| format!("read body from {url}"))?;
        if n == 0 {
            break;
        }
        if body.len() + n > max_bytes {
            body.extend_from_slice(&buf[..n.min(max_bytes.saturating_sub(body.len()))]);
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    Ok(body)
}

pub fn html_to_text(html: &str) -> String {
    let no_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let no_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let no_tags = Regex::new(r"(?s)<[^>]+>").unwrap();
    let mut text = no_script.replace_all(html, " ").into_owned();
    text = no_style.replace_all(&text, " ").into_owned();
    text = no_tags.replace_all(&text, " ").into_owned();
    collapse_whitespace(&decode_basic_entities(&text))
}

pub fn extract_title(html: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").ok()?;
    let cap = re.captures(html)?;
    let title = html_to_text(&cap[1]);
    if title.is_empty() {
        None
    } else {
        Some(title.chars().take(120).collect())
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_basic_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html_to_text() {
        let html = "<html><head><title>App Router</title></head><body><h1>Hello</h1><p>World</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("<h1>"));
    }

    #[test]
    fn extracts_title() {
        let html = "<title>Next.js Docs</title><body>x</body>";
        assert_eq!(extract_title(html).as_deref(), Some("Next.js Docs"));
    }
}
