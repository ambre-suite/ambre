//! The selective forwarding unit of a call (`ambre-spec-call-media.md`).
//!
//! It forwards the sealed chunks of a call and opens none of them. Each participant reaches it
//! over str0m with the RustCrypto backend, so no C library holds a key of the hop (`CALL-013`).

use std::sync::Arc;

use str0m::RtcConfig;

pub mod admit;
pub mod congestion;
pub mod forward;
pub mod layer;
pub mod rewrite;
pub mod secrets;
pub mod serve;
pub mod turn;

#[cfg(test)]
mod sim;
#[cfg(test)]
mod tests;

/// The connection the SFU holds to one participant.
pub fn participant_config() -> RtcConfig {
    RtcConfig::new()
        // The SFU listens on a public address: it answers connectivity checks and sends none.
        .set_ice_lite(true)
        .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
        // The pacer and the padding of each downlink follow its congestion control, which
        // reads the transport-wide feedback str0m hands over (`CALL-038`, `CALL-044`).
        .enable_bwe(Some(forward::INITIAL_ESTIMATE))
        .enable_twcc_acks(true)
}
