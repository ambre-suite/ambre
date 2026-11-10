//! The selective forwarding unit of a call (`ambre-spec-call-media.md`).
//!
//! It forwards the sealed chunks of a call and opens none of them. Each participant reaches it
//! over str0m with the RustCrypto backend, so no C library holds a key of the hop (`CALL-013`).

use std::sync::Arc;

use str0m::RtcConfig;
use str0m::format::{Codec, FormatParams};

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
    let mut config = RtcConfig::new()
        // The SFU listens on a public address: it answers connectivity checks and sends none.
        .set_ice_lite(true)
        .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
        // The pacer and the padding of each downlink follow its congestion control, which
        // reads the transport-wide feedback str0m hands over (`CALL-038`, `CALL-044`).
        .enable_bwe(Some(forward::INITIAL_ESTIMATE))
        .enable_twcc_acks(true);
    cap_audio(&mut config);
    config
}

/// Asks each sender for Opus at the constant bitrate of a call, in each session description of
/// the SFU. A 20 ms frame then fits the fixed size of a sealed chunk, and the seal drops none
/// for its size (`CALL-067`).
fn cap_audio(config: &mut RtcConfig) {
    let codecs = config.codec_config();
    let Some(opus) = codecs.find(|p| p.spec().codec == Codec::Opus).copied() else {
        return;
    };
    let spec = opus.spec();
    codecs.enable_opus(false, false);
    codecs.add_config(
        opus.pt(),
        opus.resend(),
        Codec::Opus,
        spec.clock_rate,
        spec.channels,
        FormatParams {
            max_average_bitrate: Some(ambre_core::AUDIO_BITRATE.get()),
            cbr: Some(true),
            ..spec.format
        },
    );
}
