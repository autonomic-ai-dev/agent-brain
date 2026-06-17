//! HTTPS domain allowlist for `learn_from_url`.

use anyhow::{bail, Context, Result};

pub fn default_allowed_domains() -> Vec<String> {
    vec![
        "nextjs.org".into(),
        "react.dev".into(),
        "docs.vercel.com".into(),
        "doc.rust-lang.org".into(),
        "docs.rs".into(),
        "tailwindcss.com".into(),
        "docs.cursor.com".into(),
        "developer.mozilla.org".into(),
        "typescriptlang.org".into(),
        "nodejs.org".into(),
        "docs.github.com".into(),
    ]
}

/// Parse and normalize host from an HTTPS URL.
pub fn parse_https_host(url: &str) -> Result<String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") {
        bail!("only https URLs are allowed (got non-https URL)");
    }
    let rest = trimmed
        .strip_prefix("https://")
        .context("parse https URL")?;
    let host = rest
        .split('/')
        .next()
        .filter(|h| !h.is_empty())
        .context("missing host in URL")?;
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    if host.is_empty() || host.contains('@') {
        bail!("invalid host in URL");
    }
    Ok(host)
}

pub fn domain_allowed(host: &str, allowed: &[String]) -> bool {
    let host = host.to_ascii_lowercase();
    allowed.iter().any(|entry| {
        let entry = entry.trim().trim_start_matches('.').to_ascii_lowercase();
        if entry.is_empty() {
            return false;
        }
        host == entry || host.ends_with(&format!(".{entry}"))
    })
}

pub fn assert_url_allowed(url: &str, allowed: &[String]) -> Result<String> {
    let host = parse_https_host(url)?;
    if !domain_allowed(&host, allowed) {
        bail!(
            "domain `{host}` is not in docs allowlist — add it to ~/.agent_brain/config.yaml under docs.allowed_domains"
        );
    }
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_host() {
        assert_eq!(
            parse_https_host("https://nextjs.org/docs/app").unwrap(),
            "nextjs.org"
        );
    }

    #[test]
    fn rejects_http() {
        assert!(parse_https_host("http://nextjs.org/docs").is_err());
    }

    #[test]
    fn allows_subdomain() {
        assert!(domain_allowed(
            "www.developer.mozilla.org",
            &["developer.mozilla.org".into()]
        ));
    }

    #[test]
    fn blocks_unknown_domain() {
        assert!(!domain_allowed("evil.example", &default_allowed_domains()));
    }
}
