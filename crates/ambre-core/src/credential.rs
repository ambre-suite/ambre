//! The address of an identity provider's sign-in page.
//!
//! The relay answers it for a sign-in, and the page follows it. It is no
//! secret of its own. The browser keeps it whole in the address bar and in
//! the history. The state its query carries comes back in clear in the
//! callback address. So the address is no credential: it is an `Opaque`
//! text, which a log never prints.
//!
//! The state itself is an [`crate::OidcState`], which is a secret, and the
//! code the provider answers is an [`crate::OidcCode`]. Its `Debug` still prints no byte of it,
//! so a log line keeps no sign-in state.

use crate::{OpaqueKind, OpaqueLength, WireOpaque};

/// The kind of the identity provider's authorization address the relay
/// answers for a sign-in: `Opaque<ProviderUrls>`. It is the relay's data,
/// and a page checks it before following it. It encodes as the bare string
/// it replaced. `Debug` prints the length alone: the query carries the
/// sign-in state.
pub enum ProviderUrls {}

impl OpaqueKind for ProviderUrls {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("ProviderUrl");
    const LENGTH: OpaqueLength = OpaqueLength::Bytes;
}

impl WireOpaque for ProviderUrls {}

#[cfg(test)]
mod tests {
    use super::ProviderUrls;
    use crate::Opaque;

    #[test]
    fn a_provider_address_prints_no_byte_and_serializes_as_its_text() {
        let url = Opaque::<ProviderUrls>::new("https://idp.test/auth?state=s");
        assert_eq!(format!("{url:?}"), "ProviderUrl(29 bytes)");
        assert_eq!(
            serde_json::to_string(&url).unwrap(),
            "\"https://idp.test/auth?state=s\""
        );
    }
}
