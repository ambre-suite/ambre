//! The TURN credential, against the HMAC that coturn computes.

use ambre_core::{FixedBytes, Instant, UnixSec};

use super::TurnSecret;
use crate::turn::TurnUsername;

/// `CALL-014`: the password is the HMAC-SHA1 of the username under the shared secret, in
/// base64. The reference value is the one `openssl dgst -sha1 -hmac` gives for the same pair,
/// which is what coturn checks under `use-auth-secret`. The secret prints nothing.
#[test]
fn a_turn_password_is_the_hmac_coturn_checks() {
    let secret = TurnSecret::new("north-wind".to_owned());
    let username = TurnUsername::new(
        Instant::<UnixSec>::new(1_700_000_000),
        FixedBytes::from_array([0xab; 16]),
    );
    assert_eq!(
        username.to_string(),
        "1700000000:abababababababababababababababab"
    );
    let password = secret.mint(&username);
    let wire = serde_json::to_string(&password).expect("a password travels");
    assert_eq!(wire, "\"pHDHx+9IgiI00j6T9DMMslUnN1k=\"");
    assert!(!format!("{secret:?}").contains("north"));
}
