//! The selective forwarding unit of a call (`ambre-spec-call-media.md`).
//!
//! It forwards the sealed chunks of a call and opens none of them. Each participant reaches it
//! over str0m with the RustCrypto backend, so no C library holds a key of the hop (`CALL-013`).

use std::sync::Arc;

use str0m::RtcConfig;

pub mod forward;
pub mod layer;
pub mod rewrite;
pub mod serve;

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
        // The estimate of each downlink drives the layer choice (`CALL-038`).
        .enable_bwe(Some(forward::INITIAL_ESTIMATE))
}
