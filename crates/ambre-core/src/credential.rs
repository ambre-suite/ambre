//! Texts that grant access to whoever holds them.
//!
//! A presigned URL lets its holder read or write one object, and the code and
//! state of an OIDC sign-in let their holder finish it. Each is a bearer
//! credential, so a log must never carry one. Each type's `Debug` prints no
//! byte of it. Each is `#[serde(transparent)]`, so the JSON keeps its text.

use serde::{Deserialize, Serialize};

macro_rules! define_credential {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wraps the credential text.
            pub fn new(text: String) -> Self {
                Self(text)
            }

            /// The credential text, for the one call that presents it.
            pub fn expose(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }
    };
}

define_credential! {
    /// A presigned object-store URL: it reads or writes one object until it
    /// expires.
    PresignedUrl
}

define_credential! {
    /// The authorization code an identity provider hands back to the client.
    OidcCode
}

define_credential! {
    /// The CSRF state that binds a sign-in callback to the sign-in it began.
    OidcState
}

#[cfg(test)]
mod tests {
    use super::PresignedUrl;

    #[test]
    fn a_credential_prints_no_byte_and_serializes_as_its_text() {
        let url = PresignedUrl::new("https://store.test/o?sig=secret".to_owned());
        assert_eq!(format!("{url:?}"), "PresignedUrl(<redacted>)");
        assert_eq!(
            serde_json::to_string(&url).unwrap(),
            "\"https://store.test/o?sig=secret\""
        );
    }
}
