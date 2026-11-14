//! The chrome rules the judge settles: one over the graph `cargo metadata`
//! resolves, and one over a fact the driver records.
//!
//! `crate_boundaries.py` read these on the text of a manifest line and of a
//! function body. So a renamed dependency, a feature that named the crate and
//! a create a helper built passed unseen.
//!
//! | Check | Row |
//! | :--- | :--- |
//! | the crypto crate declared outside the worker, the call worker and the Logic fixtures | `CHROME-049` |
//! | the open-time create recorded by no unit | `CHROME-037` |

use super::{Activation, FIXTURES, Graph, Records};

/// The crate that hashes a key (`CHROME-049`).
const CRYPTO: &str = "ambre-crypto";

/// The crates that hold the keys, and so declare the crypto crate. The call
/// worker seals and opens the chunks of a call (`CHROME-049`, `CALL-003`).
const CRYPTO_HOLDERS: &[&str] = &["ambre-sync", "ambre-call"];

/// The crate that may declare the crypto crate for its fixtures alone. Its
/// fixture feature alone may turn it on (`CHROME-049`).
const CRYPTO_FIXTURES: &str = "ambre-logic";

/// The fact the driver records for the open-time create that sends no name
/// (`CHROME-037`).
pub(crate) const LAZY_CREATE: &str = "lazy-create-unnamed";

/// Whether a feature entry turns on the crypto crate: `dep:ambre-crypto`,
/// `ambre-crypto` or `ambre-crypto/x`.
fn names_crypto(entry: &str) -> bool {
    let entry = entry.strip_prefix("dep:").unwrap_or(entry);
    entry.split(['/', '?']).next() == Some(CRYPTO)
}

/// Each refusal of `CHROME-049` over the graph. The chrome hashes nothing, so
/// a fingerprint arrives formatted. A member declares the crypto crate only
/// when it holds the keys. The Logic crate declares it as an optional
/// dependency, which its fixture feature alone turns on. Cargo undoes a
/// rename, so the declared name is the package name.
pub(crate) fn manifests(graph: &Graph) -> Vec<String> {
    let mut refusals = Vec::new();
    for id in &graph.members {
        let Some(p) = graph.packages.get(id) else {
            continue;
        };
        if p.name == CRYPTO || CRYPTO_HOLDERS.contains(&p.name.as_str()) {
            continue;
        }
        let fixtures = p.name == CRYPTO_FIXTURES;
        for d in p.declared.iter().filter(|d| d.name == CRYPTO) {
            if !(fixtures && d.activation == Activation::Feature && d.kind.is_none()) {
                refusals.push(format!(
                    "{}: {} declares the crypto crate outside the worker, so the chrome could \
                     hash a key (CHROME-049)",
                    p.manifest, p.name
                ));
            }
        }
        if fixtures {
            for (feature, entries) in &p.features {
                if feature != FIXTURES && entries.iter().any(|e| names_crypto(e)) {
                    refusals.push(format!(
                        "{}: the feature `{feature}` turns on the crypto crate, which only \
                         `{FIXTURES}` may (CHROME-049)",
                        p.manifest
                    ));
                }
            }
        }
    }
    refusals
}

/// The refusal of `CHROME-037` when no unit records the open-time create. The
/// driver refuses a create that sends a name where it reads it. A create no
/// unit reads would hold the rule by reading nothing, so it fails closed.
pub(crate) fn facts(records: &Records) -> Vec<String> {
    if records.facts.iter().any(|(name, _, _)| name == LAZY_CREATE) {
        return Vec::new();
    }
    vec![
        "the driver records no open-time create in `trigger_lazy_create`, so the unnamed \
         create reads nothing, and fails closed (CHROME-037)"
            .to_owned(),
    ]
}

#[cfg(test)]
#[path = "chrome_tests.rs"]
mod tests;
