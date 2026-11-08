//! The unwrap check (`ARCH-073`), on the approximations the syn report made:
//! each site the driver must judge with types.

use crate::harness::{Build, Workspace, of};

const FIXTURE: &str = r#"
pub mod names {
    pub struct Name(String);
    impl Name {
        pub fn as_str(&self) -> &str { &self.0 }
    }
    pub struct Root<K>(String, core::marker::PhantomData<K>);
    impl<K> Root<K> {
        pub fn parse(text: &str) -> Option<Self> { Some(Self(text.to_owned(), core::marker::PhantomData)) }
    }
    pub fn inside(name: &Name) { crate::a::store(name.as_str()) }
}
pub mod a {
    pub struct Key(String);
    impl Key { pub fn as_str(&self) -> &str { &self.0 } }
    pub fn store(_: &str) {}
}
pub mod b {
    pub struct Key(u32);
}
pub mod sites {
    use crate::names::{Name, Root};
    pub type MarkRoot = Root<u8>;
    // Through an alias of a generic type.
    pub fn alias(name: &Name) -> bool { MarkRoot::parse(name.as_str()).is_some() }
    // On a receiver two globs leave untyped for a reader of names.
    use crate::a::*;
    #[allow(unused_imports)]
    use crate::b::*;
    pub fn glob(key: &crate::a::Key) { store(key.as_str()) }
    // A method of the primitive is a foreign call.
    pub fn foreign(name: &Name) -> usize { name.as_str().chars().count() }
    // Kept in a binding, then handed on.
    pub fn binding(name: &Name) { let bare = name.as_str(); crate::a::store(bare); }
}
"#;

// Each refusal carries the key `ambre-unwrap.tsv` names; a foreign call and
// the module of the newtype pass (`ARCH-073`).
#[test]
fn an_unwrap_handed_to_a_workspace_callee_is_refused() {
    let ws = Workspace::new("unwrap");
    ws.file("crates/ambre-demo/src/lib.rs", FIXTURE);
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let keys: Vec<String> = of(&records, "unwrap")
        .iter()
        .filter_map(|l| l.split('\t').next())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        keys,
        [
            "ambre-demo | src/lib.rs | crate::sites::alias | Name::as_str -> MarkRoot::parse",
            "ambre-demo | src/lib.rs | crate::sites::binding | Name::as_str -> crate::a::store",
            "ambre-demo | src/lib.rs | crate::sites::glob | Key::as_str -> store",
        ]
    );
}

// Test code is out of the check.
#[test]
fn test_code_is_not_read() {
    let ws = Workspace::new("unwrap-test");
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        &format!("{FIXTURE}\n#[cfg(test)]\nmod t {{ fn f(n: &crate::names::Name) {{ crate::a::store(n.as_str()) }} }}\n"),
    );
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Test);
    assert_eq!(of(&records, "unwrap"), Vec::<String>::new());
}
