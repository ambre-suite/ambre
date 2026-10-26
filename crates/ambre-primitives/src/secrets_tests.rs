use super::refusals;

/// The refusals of one file, each cut to its place and its name.
fn refused(krate: &str, relative: &str, source: &str) -> Vec<String> {
    let tree = match syn::parse_file(source) {
        Ok(tree) => tree,
        Err(e) => panic!("the test source does not parse: {e}"),
    };
    refusals(
        krate,
        relative,
        &format!("crates/{krate}/{relative}"),
        &tree,
    )
    .into_iter()
    .map(|refusal| match refusal.split_once(" is named here") {
        Some((head, _)) => head.to_owned(),
        None => refusal,
    })
    .collect()
}

// The red cases are the code of 66bec579, before Secret.

#[test]
fn the_token_of_the_sync_worker_at_66bec579_is_refused() {
    let found = refused(
        "ambre-sync",
        "src/http.rs",
        "use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};\n\
         #[derive(Clone, Default, Deserialize, Serialize, Zeroize, ZeroizeOnDrop)]\n\
         pub(crate) struct SecretToken(String);\n\
         impl SecretToken {\n\
             pub(crate) fn expose(&self) -> &str { &self.0 }\n\
         }\n\
         pub(crate) fn bearer(token: &SecretToken) -> Zeroizing<String> {\n\
             Zeroizing::new(format!(\"Bearer {}\", token.expose()))\n\
         }",
    );
    assert_eq!(
        found,
        [
            "crates/ambre-sync/src/http.rs:1: `zeroize`",
            "crates/ambre-sync/src/http.rs:1: `Zeroize`",
            "crates/ambre-sync/src/http.rs:1: `ZeroizeOnDrop`",
            "crates/ambre-sync/src/http.rs:1: `Zeroizing`",
            "crates/ambre-sync/src/http.rs:2: `Zeroize`",
            "crates/ambre-sync/src/http.rs:2: `ZeroizeOnDrop`",
            "crates/ambre-sync/src/http.rs:5: `expose`",
            "crates/ambre-sync/src/http.rs:7: `Zeroizing`",
            "crates/ambre-sync/src/http.rs:8: `Zeroizing`",
            "crates/ambre-sync/src/http.rs:8: `expose`",
        ]
    );
}

#[test]
fn the_seed_text_of_the_relay_at_66bec579_is_refused() {
    let found = refused(
        "ambre-server",
        "src/main.rs",
        "fn main() {\n\
             let seed = jwt::JwtSeed::from_hex(&zeroize::Zeroizing::new(hex_seed));\n\
         }",
    );
    assert_eq!(
        found,
        [
            "crates/ambre-server/src/main.rs:2: `zeroize`",
            "crates/ambre-server/src/main.rs:2: `Zeroizing`",
        ]
    );
}

#[test]
fn a_wipe_derive_is_refused_in_ambre_crypto_too() {
    let found = refused(
        "ambre-crypto",
        "src/keys.rs",
        "#[derive(Zeroize, ZeroizeOnDrop)]\npub struct MasterSeed([u8; 32]);",
    );
    assert_eq!(
        found,
        [
            "crates/ambre-crypto/src/keys.rs:1: `Zeroize`",
            "crates/ambre-crypto/src/keys.rs:1: `ZeroizeOnDrop`",
        ]
    );
}

#[test]
fn a_manual_wipe_is_refused_through_its_trait_and_its_call() {
    let found = refused(
        "ambre-bus",
        "src/rest.rs",
        "impl Drop for Bytes {\n\
             fn drop(&mut self) { self.0.zeroize(); }\n\
         }\n\
         impl zeroize::ZeroizeOnDrop for Bytes {}",
    );
    assert_eq!(
        found,
        [
            "crates/ambre-bus/src/rest.rs:2: `zeroize`",
            "crates/ambre-bus/src/rest.rs:4: `zeroize`",
            "crates/ambre-bus/src/rest.rs:4: `ZeroizeOnDrop`",
        ]
    );
}

#[test]
fn the_crypto_crate_reads_a_secret_and_the_secret_module_wipes_one() {
    let read = "fn key(seed: &Seed) -> [u8; 32] { *seed.0.expose() }\n\
                fn copy(seed: &Seed) -> Seed { Seed(seed.0.clone_secret()) }";
    assert_eq!(
        refused("ambre-crypto", "src/keys.rs", read),
        Vec::<String>::new()
    );
    let module = "use zeroize::{Zeroize, ZeroizeOnDrop};\n\
                  impl<T: Zeroize> Drop for Secret<T> { fn drop(&mut self) { self.0.zeroize(); } }\n\
                  impl<T: Zeroize> Secret<T> { pub fn expose(&self) -> &T { &self.0 } }";
    assert_eq!(
        refused("ambre-core", "src/secret.rs", module),
        Vec::<String>::new()
    );
}

#[test]
fn another_file_of_ambre_core_is_no_secret_module() {
    let found = refused(
        "ambre-core",
        "src/credential.rs",
        "fn text(url: &Url) -> &str { url.0.expose() }",
    );
    assert_eq!(found, ["crates/ambre-core/src/credential.rs:1: `expose`"]);
}

#[test]
fn every_reader_is_refused_outside_ambre_crypto_in_each_form() {
    let found = refused(
        "ambre-sync",
        "src/auth.rs",
        "use ambre_core::Secret;\n\
         fn a(s: &Secret<String>) -> &String { Secret::expose(s) }\n\
         fn b(s: &mut Secret<String>) -> &mut String { s.expose_mut() }\n\
         fn c(s: &Secret<String>) -> Secret<String> { s.clone_secret() }\n\
         fn d(s: &Secret<String>) -> &String { s.r#expose() }\n\
         use ambre_core::Secret as S;\n\
         use other::expose as show;",
    );
    assert_eq!(
        found,
        [
            "crates/ambre-sync/src/auth.rs:2: `expose`",
            "crates/ambre-sync/src/auth.rs:3: `expose_mut`",
            "crates/ambre-sync/src/auth.rs:4: `clone_secret`",
            "crates/ambre-sync/src/auth.rs:5: `expose`",
            "crates/ambre-sync/src/auth.rs:7: `expose`",
        ]
    );
}

#[test]
fn a_name_inside_a_macro_or_an_attribute_is_refused_and_a_string_is_not() {
    let found = refused(
        "ambre-app",
        "src/lib.rs",
        "fn log(s: &Secret<String>) { tracing::info!(\"{}\", wrap(s.expose())); }\n\
         #[derive(Serialize)]\n\
         struct Row { #[serde(serialize_with = \"show\")] key: Key }\n\
         #[derive(Serialize)]\n\
         struct Out { #[serde(serialize_with = expose)] key: Key }",
    );
    assert_eq!(
        found,
        [
            "crates/ambre-app/src/lib.rs:1: `expose`",
            "crates/ambre-app/src/lib.rs:5: `expose`",
        ]
    );
}

#[test]
fn test_code_is_never_refused() {
    let found = refused(
        "ambre-sync",
        "src/http.rs",
        "#[cfg(test)]\n\
         mod tests {\n\
             use zeroize::Zeroize;\n\
             #[test]\n\
             fn wipes() { token.0.expose(); }\n\
         }\n\
         #[test]\n\
         fn alone() { token.0.expose(); }\n\
         struct T;\n\
         impl T {\n\
             #[cfg(test)]\n\
             fn probe(&self) -> &str { self.0.expose() }\n\
         }",
    );
    assert_eq!(found, Vec::<String>::new());
    let file = refused(
        "ambre-sync",
        "src/probe.rs",
        "#![cfg(test)]\nfn wipes() { token.0.expose(); }",
    );
    assert_eq!(file, Vec::<String>::new());
}

#[test]
fn another_build_of_the_code_is_refused() {
    let found = refused(
        "ambre-sync",
        "src/http.rs",
        "#[cfg(target_arch = \"wasm32\")]\nfn send(t: &Token) { t.0.expose(); }",
    );
    assert_eq!(found, ["crates/ambre-sync/src/http.rs:2: `expose`"]);
}

#[test]
fn a_refusal_names_its_rule() {
    let tree =
        match syn::parse_file("fn a(s: &S) { s.expose(); let _ = zeroize::Zeroizing::new(1); }") {
            Ok(tree) => tree,
            Err(e) => panic!("the test source does not parse: {e}"),
        };
    let found = refusals(
        "ambre-bus",
        "src/lib.rs",
        "crates/ambre-bus/src/lib.rs",
        &tree,
    );
    assert_eq!(found.len(), 3);
    assert!(found[0].contains("(ARCH-045)"), "{}", found[0]);
    assert!(found[1].contains("(ARCH-046)"), "{}", found[1]);
    assert!(found[2].contains("(ARCH-046)"), "{}", found[2]);
}

#[test]
fn a_dependency_that_renames_zeroize_is_refused() {
    let renamed = std::collections::BTreeMap::from([
        ("wipe".to_owned(), "zeroize".to_owned()),
        ("cbor".to_owned(), "ciborium".to_owned()),
    ]);
    let found = super::renamed_wipers("ambre-sync", &renamed);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].starts_with("crates/ambre-sync/Cargo.toml: `wipe` renames the zeroize crate"),
        "{}",
        found[0]
    );
}
