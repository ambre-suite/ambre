//! The token a scrape of the metrics presents, as the relay reads it.

use ambre_core::{Credential, ScrapeToken};

/// The token `METRICS_TOKEN` sets, or `None` when it is unset or empty. The
/// text goes straight into its credential, and the scrape compares it there
/// in constant time (`SRV-307`).
pub fn scrape_token() -> Option<Credential<ScrapeToken>> {
    std::env::var("METRICS_TOKEN")
        .ok()
        .filter(|text| !text.is_empty())
        .map(Credential::new)
}
