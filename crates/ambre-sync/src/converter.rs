//! The converter module fetch (`ambre-spec-architecture.md section 2`, the
//! converter module).
//!
//! The Logic thread cannot fetch, and every network call of the client leaves
//! from this worker (`ARCH-005`). So the worker fetches the module from the
//! page's origin. It checks the bytes against the BLAKE2b-256 digest this
//! build carries before any reach the Logic thread (`ARCH-080`). The digest is
//! the module's name too, so a cached copy of another release never answers.
//!
//! The build writes the digest (`scripts/converter_build.py`), and
//! `build.rs` hands it to the compiler. A build made without it, such as a
//! bare `cargo build`, carries no converter and says so.

use ambre_core::{ConverterModule, FixedBytes, ModuleDigest, Payload, PayloadKind};

use crate::SyncState;

/// The digest this build carries, as the build script wrote it in hex.
const DIGEST: Option<&str> = option_env!("AMBRE_CONVERTER_DIGEST");

/// The hex digits of the digest the module's file name carries. Sixteen
/// digits name a release uniquely; the whole digest checks the bytes.
const NAME_DIGITS: usize = 16;

/// The role of the bytes the fetch brought, before their digest is checked.
/// `Payload<FetchedModule>`: only [`check`] turns them into a
/// `Payload<ConverterModule>`.
pub(crate) enum FetchedModule {}

impl PayloadKind for FetchedModule {
    const NAME: &'static str = "FetchedModule";
}

/// Checked, the fetched bytes are the module (`check`).
impl ambre_core::Becomes<ConverterModule> for FetchedModule {}

/// The digest this build carries, or why there is none.
fn expected() -> Result<FixedBytes<ModuleDigest>, String> {
    let hex = DIGEST.ok_or_else(|| "this build carries no converter".to_owned())?;
    FixedBytes::from_hex(hex)
        .map_err(|e| format!("the build's converter digest is unreadable: {e}"))
}

/// The address of the module beside the worker's own script: the same origin,
/// named by its digest (`scripts/trunk_post_build.py`).
pub(crate) fn module_path(digest: &FixedBytes<ModuleDigest>) -> String {
    let hex = digest.to_hex();
    format!(
        "./ambre-convert-{}.wasm",
        hex.get(..NAME_DIGITS).unwrap_or(&hex)
    )
}

/// The fetched bytes, accepted only when their digest is `expected`.
///
/// # Errors
///
/// The reason, in words the Logic thread shows, when the digests differ.
pub(crate) fn check(
    bytes: Payload<FetchedModule>,
    expected: &FixedBytes<ModuleDigest>,
) -> Result<Payload<ConverterModule>, String> {
    let got = ambre_crypto::hash::module_digest(&bytes);
    if &got == expected {
        Ok(bytes.into_role())
    } else {
        Err("the converter fetched is not the one this build names".to_owned())
    }
}

impl SyncState {
    /// Fetch the module, then answer on the loop (`SyncCommand::FetchConverter`).
    pub(crate) fn handle_fetch_converter(&self) {
        let digest = match expected() {
            Ok(digest) => digest,
            Err(reason) => {
                return self.send_to_logic(ambre_bus::LogicEvent::ConverterRefused { reason });
            }
        };
        let Some(tx) = self.event_tx.clone() else {
            return self.send_to_logic(ambre_bus::LogicEvent::ConverterRefused {
                reason: "this transport fetches nothing".to_owned(),
            });
        };
        let path = module_path(&digest);
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch(&path).await;
            let _ = tx.send(crate::SyncEvent::ConverterFetched { result });
        });
    }

    /// The fetched bytes, on the loop: checked, then handed to the Logic
    /// thread, or refused with the reason.
    pub(crate) fn converter_fetched(&self, result: Result<Payload<FetchedModule>, String>) {
        let answer = result.and_then(|bytes| check(bytes, &expected()?));
        self.send_to_logic(match answer {
            Ok(module) => ambre_bus::LogicEvent::ConverterFetched { module },
            Err(reason) => ambre_bus::LogicEvent::ConverterRefused { reason },
        });
    }
}

/// The bytes at `path`, or the reason there are none.
async fn fetch(path: &str) -> Result<Payload<FetchedModule>, String> {
    let response = gloo_net::http::Request::get(path)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(format!("the server answered {}", response.status()));
    }
    response
        .binary()
        .await
        .map(Payload::new)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_of_another_digest_never_reach_the_logic_thread() {
        let module = b"\0asm\x01\0\0\0".to_vec();
        let digest =
            ambre_crypto::hash::module_digest(&Payload::<FetchedModule>::new(module.clone()));
        assert!(check(Payload::new(module.clone()), &digest).is_ok());
        let mut tampered = module;
        tampered.push(0);
        assert!(check(Payload::new(tampered), &digest).is_err());
    }

    #[test]
    fn the_module_is_named_by_its_digest_beside_the_worker() {
        let digest =
            ambre_crypto::hash::module_digest(&Payload::<FetchedModule>::new(b"abc".to_vec()));
        assert_eq!(
            module_path(&digest),
            "./ambre-convert-bddd813c63423972.wasm"
        );
    }
}
