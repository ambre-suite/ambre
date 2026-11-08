//! The names that read, wipe or lend a secret, held to the files that may
//! write them (`ARCH-045`, `ARCH-046`, `ARCH-048`, `ARCH-049`).

use crate::harness::{Build, Workspace, of};

/// The refusals of `check` in `records`.
fn refused(records: &[String], check: &str) -> Vec<String> {
    of(records, "refuse")
        .into_iter()
        .filter_map(|l| l.strip_prefix(&format!("{check}\t")).map(str::to_owned))
        .collect()
}

/// A Secret module, as `ambre-core` writes it, with its readers and a sink.
const SECRET: &str = "pub struct Secret(Vec<u8>);\n\
    impl Secret {\n\
        pub fn expose(&self) -> &[u8] { &self.0 }\n\
        pub fn clone_secret(&self) -> Secret { Secret(self.0.clone()) }\n\
        pub fn present_bearer(&self, read: impl FnOnce(&[u8])) { read(&self.0) }\n\
    }\n";

// A reader is named in `ambre-crypto`, the Secret module and the relay's
// secrets module alone; anywhere else it is refused, called or declared
// (`ARCH-045`, `ARCH-048`).
#[test]
fn every_reader_is_refused_outside_the_modules_that_may_read() {
    let ws = Workspace::new("readers");
    ws.file("crates/ambre-core/src/lib.rs", "pub mod secret;\n");
    ws.file("crates/ambre-core/src/secret.rs", SECRET);
    ws.file(
        "crates/ambre-crypto/src/lib.rs",
        "pub fn open(s: &ambre_core::secret::Secret) -> usize { s.expose().len() }\n",
    );
    ws.file(
        "crates/ambre-server/src/lib.rs",
        "pub mod secrets;\npub fn leak(s: &ambre_core::secret::Secret) -> usize { s.expose().len() }\n",
    );
    ws.file(
        "crates/ambre-server/src/secrets/mod.rs",
        "pub fn read(s: &ambre_core::secret::Secret) -> usize { s.clone_secret().expose().len() }\n",
    );
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "pub struct Other;\nimpl Other { pub fn expose_mut(&self) {} }\n\
         pub fn f(s: &ambre_core::secret::Secret, o: &Other) { let _ = s.expose(); o.expose_mut(); }\n",
    );
    let core = ws.compile("ambre_core", "ambre-core", &[], Build::Shipped);
    assert_eq!(refused(&core, "secret"), Vec::<String>::new());
    let crypto = ws.compile(
        "ambre_crypto",
        "ambre-crypto",
        &["ambre_core"],
        Build::Shipped,
    );
    assert_eq!(refused(&crypto, "secret"), Vec::<String>::new());
    let server = ws.compile(
        "ambre_server",
        "ambre-server",
        &["ambre_core"],
        Build::Shipped,
    );
    let server = refused(&server, "secret");
    assert_eq!(server.len(), 1, "{server:#?}");
    assert!(
        server[0].starts_with("crates/ambre-server/src/lib.rs:2: expose"),
        "{server:#?}"
    );
    let demo = ws.compile("ambre_demo", "ambre-demo", &["ambre_core"], Build::Shipped);
    let demo = refused(&demo, "secret");
    // The declaration of `expose_mut`, its call, and the call of `expose`.
    assert_eq!(demo.len(), 3, "{demo:#?}");
}

// The `zeroize` crate is named in the Secret module alone, through a path, a
// trait, a derive or a method (`ARCH-046`).
#[test]
fn a_name_of_the_zeroize_crate_is_refused_outside_the_secret_module() {
    let ws = Workspace::new("zeroize");
    ws.file(
        "crates/zeroize/src/lib.rs",
        "pub trait Zeroize { fn zeroize(&mut self); }\nimpl Zeroize for Vec<u8> { fn zeroize(&mut self) { self.clear() } }\n",
    );
    ws.file("crates/ambre-core/src/lib.rs", "pub mod secret;\n");
    ws.file(
        "crates/ambre-core/src/secret.rs",
        "use zeroize::Zeroize;\npub fn wipe(v: &mut Vec<u8>) { v.zeroize() }\n",
    );
    ws.file(
        "crates/ambre-crypto/src/lib.rs",
        "use zeroize::Zeroize;\npub fn wipe(v: &mut Vec<u8>) { v.zeroize() }\n",
    );
    ws.compile("zeroize", "zeroize", &[], Build::Shipped);
    let core = ws.compile("ambre_core", "ambre-core", &["zeroize"], Build::Shipped);
    assert_eq!(refused(&core, "zeroize"), Vec::<String>::new());
    let crypto = ws.compile("ambre_crypto", "ambre-crypto", &["zeroize"], Build::Shipped);
    assert!(refused(&crypto, "zeroize").len() >= 2, "{crypto:#?}");
}

// A sink is named in the file of its one foreign call alone, beside the
// Secret module that declares it (`ARCH-049`).
#[test]
fn a_sink_is_named_in_the_file_that_makes_its_call_alone() {
    let ws = Workspace::new("sinks");
    ws.file("crates/ambre-core/src/lib.rs", "pub mod secret;\n");
    ws.file("crates/ambre-core/src/secret.rs", SECRET);
    ws.file(
        "crates/ambre-sync/src/lib.rs",
        "pub mod http;\npub fn elsewhere(s: &ambre_core::secret::Secret) { s.present_bearer(|_| ()) }\n",
    );
    ws.file(
        "crates/ambre-sync/src/http.rs",
        "pub fn send(s: &ambre_core::secret::Secret) { s.present_bearer(|_| ()) }\n",
    );
    ws.compile("ambre_core", "ambre-core", &[], Build::Shipped);
    let sync = ws.compile("ambre_sync", "ambre-sync", &["ambre_core"], Build::Shipped);
    let sinks = refused(&sync, "sink");
    assert_eq!(sinks.len(), 1, "{sinks:#?}");
    assert!(
        sinks[0].starts_with("crates/ambre-sync/src/lib.rs:2: the sink present_bearer"),
        "{sinks:#?}"
    );
}

// A test build reads no name: a test may read a secret (`ARCH-045`).
#[test]
fn test_code_reads_a_secret_freely() {
    let ws = Workspace::new("secret-test");
    ws.file("crates/ambre-core/src/lib.rs", "pub mod secret;\n");
    ws.file("crates/ambre-core/src/secret.rs", SECRET);
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "#[cfg(test)]\nmod tests { #[test] fn t() { let s = ambre_core::secret::Secret::default_for_test(); } }\n",
    );
    ws.file(
        "crates/ambre-core/src/secret.rs",
        &format!(
            "{SECRET}impl Secret {{ pub fn default_for_test() -> Self {{ Secret(Vec::new()) }} }}\n"
        ),
    );
    ws.compile("ambre_core", "ambre-core", &[], Build::Shipped);
    let test = ws.compile("ambre_demo", "ambre-demo", &["ambre_core"], Build::Test);
    assert_eq!(refused(&test, "secret"), Vec::<String>::new());
}
