//! The converter module fetch (`ambre-spec-architecture.md section 2`, the
//! converter modules).
//!
//! The Logic thread cannot fetch, and every network call of the client leaves
//! from this worker (`ARCH-005`). So the worker fetches a family's module from
//! the page's origin. It checks the bytes against the BLAKE2b-256 digest this
//! build carries for that family before any reach the Logic thread
//! (`ARCH-080`). The digest is the module's name too, so a cached copy of
//! another release never answers.
//!
//! The build writes one digest per family (`scripts/converter_build.py`), and
//! `build.rs` hands each to the compiler. A build made without them, such as
//! a bare `cargo build`, carries no converter and says so.

use ambre_core::{
    ConverterFamily, ConverterModule, ConverterRefusal, FixedBytes, ModuleDigest, Payload,
    PayloadKind,
};

use crate::SyncState;

/// The digest this build carries for `family`, as the build script wrote it
/// in hex. A family that joins the list adds its arm, and the match holds
/// every family to one.
const fn digest_hex(family: ConverterFamily) -> Option<&'static str> {
    match family {
        ConverterFamily::Tables => option_env!("AMBRE_CONVERTER_DIGEST_TABLES"),
        ConverterFamily::Pdf => option_env!("AMBRE_CONVERTER_DIGEST_PDF"),
        ConverterFamily::Markup => option_env!("AMBRE_CONVERTER_DIGEST_MARKUP"),
        ConverterFamily::Images => option_env!("AMBRE_CONVERTER_DIGEST_IMAGES"),
        ConverterFamily::Office => option_env!("AMBRE_CONVERTER_DIGEST_OFFICE"),
    }
}

/// The hex digits of the digest the module's file name carries. Sixteen
/// digits name a release uniquely; the whole digest checks the bytes.
const NAME_DIGITS: usize = 16;

/// The role of the bytes the fetch brought, before their digest is checked.
/// `Payload<FetchedModule>`: only [`check`] turns them into a
/// `Payload<ConverterModule>`.
pub(crate) enum FetchedModule {}

impl PayloadKind for FetchedModule {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("FetchedModule");
}

/// Checked, the fetched bytes are the module (`check`).
impl ambre_core::Becomes<ConverterModule> for FetchedModule {}

/// The digest this build carries for `family`, or why there is none.
fn expected(family: ConverterFamily) -> Result<FixedBytes<ModuleDigest>, ConverterRefusal> {
    let hex = digest_hex(family).ok_or_else(|| {
        tracing::warn!("Sync: this build carries no {family} converter");
        ConverterRefusal::NoDigest
    })?;
    FixedBytes::from_hex(hex).map_err(|e| {
        tracing::warn!("Sync: the build's {family} converter digest is unreadable: {e}");
        ConverterRefusal::NoDigest
    })
}

/// The address of a family's module beside the worker's own script: the same
/// origin, named by its family and its digest (`scripts/trunk_post_build.py`).
pub(crate) fn module_path(family: ConverterFamily, digest: &FixedBytes<ModuleDigest>) -> String {
    let hex = digest.to_hex();
    format!(
        "./ambre-convert-{}-{}.wasm",
        family.name(),
        hex.get(..NAME_DIGITS).unwrap_or(&hex)
    )
}

/// The fetched bytes, accepted only when their digest is `expected`.
///
/// # Errors
///
/// [`ConverterRefusal::DigestMismatch`] when the digests differ.
pub(crate) fn check(
    bytes: Payload<FetchedModule>,
    expected: &FixedBytes<ModuleDigest>,
) -> Result<Payload<ConverterModule>, ConverterRefusal> {
    let got = ambre_crypto::hash::module_digest(&bytes);
    if &got == expected {
        Ok(bytes.into_role())
    } else {
        tracing::warn!("Sync: the converter fetched is not the one this build names");
        Err(ConverterRefusal::DigestMismatch)
    }
}

impl SyncState {
    /// Fetch the module of `family`, then answer on the loop
    /// (`SyncCommand::FetchConverter`).
    pub(crate) fn handle_fetch_converter(&self, family: ConverterFamily) {
        let digest = match expected(family) {
            Ok(digest) => digest,
            Err(cause) => {
                return self
                    .send_to_logic(ambre_bus::LogicEvent::ConverterRefused { family, cause });
            }
        };
        let Some(tx) = self.event_tx.clone() else {
            return self.send_to_logic(ambre_bus::LogicEvent::ConverterRefused {
                family,
                cause: ConverterRefusal::NoFetch,
            });
        };
        let path = module_path(family, &digest);
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch(&path).await;
            let _ = tx.send(crate::SyncEvent::ConverterFetched { family, result });
        });
    }

    /// The fetched bytes of `family`, on the loop: checked against that
    /// family's digest, then handed to the Logic thread, or refused with the
    /// reason.
    pub(crate) fn converter_fetched(
        &self,
        family: ConverterFamily,
        result: Result<Payload<FetchedModule>, ConverterRefusal>,
    ) {
        let answer = result.and_then(|bytes| check(bytes, &expected(family)?));
        self.send_to_logic(match answer {
            Ok(module) => ambre_bus::LogicEvent::ConverterFetched { family, module },
            Err(cause) => ambre_bus::LogicEvent::ConverterRefused { family, cause },
        });
    }
}

/// The bytes at `path`, or [`ConverterRefusal::NotFetched`]. The cause goes
/// to the log here.
async fn fetch(path: &str) -> Result<Payload<FetchedModule>, ConverterRefusal> {
    let not_fetched = |e: &dyn std::fmt::Display| {
        tracing::warn!("Sync: the converter at {path} did not download: {e}");
        ConverterRefusal::NotFetched
    };
    let response = gloo_net::http::Request::get(path)
        .send()
        .await
        .map_err(|e| not_fetched(&e))?;
    if !response.ok() {
        return Err(not_fetched(&response.status()));
    }
    response
        .binary()
        .await
        .map(Payload::new)
        .map_err(|e| not_fetched(&e))
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
            module_path(ConverterFamily::Markup, &digest),
            "./ambre-convert-markup-bddd813c63423972.wasm"
        );
    }

    /// Each family names its own file, so a module of one family never
    /// answers a fetch of another.
    #[test]
    fn each_family_names_a_file_of_its_own() {
        let digest =
            ambre_crypto::hash::module_digest(&Payload::<FetchedModule>::new(b"abc".to_vec()));
        let paths: std::collections::BTreeSet<String> = ConverterFamily::ALL
            .iter()
            .map(|family| module_path(*family, &digest))
            .collect();
        assert_eq!(paths.len(), ConverterFamily::ALL.len());
    }
}
