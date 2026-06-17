mod allowlist;
mod fetch;
mod learn;

pub use allowlist::{assert_url_allowed, default_allowed_domains, domain_allowed, parse_https_host};
pub use fetch::{extract_title, fetch_url, html_to_text};
pub use learn::{learn_from_url, learn_with_settings, LearnReport};
