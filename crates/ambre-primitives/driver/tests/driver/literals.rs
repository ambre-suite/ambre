//! The `bool` literals each call passes, and the parameter each reaches
//! (`ARCH-040`, `ARCH-041`).

use crate::harness::{Build, Workspace, of, site};

const LIB: &str = r#"
pub fn set(on: bool) {}
pub fn maybe(on: Option<bool>) {}
pub fn pair(p: (bool, u8)) {}
pub struct Px;
impl Px { pub fn call(&self, on: bool) {} }
pub trait Toggle { fn toggle(&self, on: bool); }
impl Toggle for Px { fn toggle(&self, on: bool) {} }
pub mod deep { pub fn set(on: bool) {} }
pub use deep::set as renamed;
"#;

const CALLS: &str = r#"
use ambre_lib::{Px, Toggle, renamed, set, maybe, pair};
pub fn calls(px: &Px, c: bool) {
    set(true);
    set(!false);
    set({ true });
    set(if c { true } else { c });
    set(match c { _ => false });
    maybe(Some(true));
    pair((true, 1));
    px.call(true);
    Px::call(px, false);
    px.toggle(true);
    renamed(true);
    set(c);
    let f = set;
    f(true);
    set(matches!(c, true));
    maybe(Some(matches!(c, false)));
    let v = vec![true];
}
"#;

/// The slots the literals of the `calls` crate reach.
fn slots() -> Vec<String> {
    let ws = Workspace::new("literals");
    ws.file("crates/ambre-lib/src/lib.rs", LIB);
    ws.file("crates/ambre-calls/src/lib.rs", CALLS);
    ws.compile("ambre_lib", "ambre-lib", &[], Build::Shipped);
    let records = ws.compile("ambre_calls", "ambre-calls", &["ambre_lib"], Build::Shipped);
    of(&records, "lit")
        .iter()
        .filter_map(|line| line.split('\t').next())
        .map(str::to_owned)
        .collect()
}

// A literal counts bare, behind `!`, in a block of one expression, in a
// branch and in an arm. It counts under `Some` and in a tuple (`ARCH-040`).
#[test]
fn a_literal_is_found_in_each_shape_the_rule_counts() {
    let slots = slots();
    let count = |slot: &str| slots.iter().filter(|s| *s == slot).count();
    assert_eq!(count("ambre_lib::set#0"), 5, "{slots:#?}");
    // `matches!` writes its own `true` and `false`: no literal of the caller.
    assert_eq!(count("ambre_lib::maybe#0"), 1, "{slots:#?}");
    assert_eq!(count("ambre_lib::pair#0"), 1, "{slots:#?}");
}

// The driver resolves each call. A method call and a path call of one method
// reach one slot, `self` left out. A trait method reaches the trait item and
// the impl. A `pub use ... as` reaches the `fn` it renames (`ARCH-040`).
#[test]
fn a_literal_reaches_the_fn_its_call_resolves_to() {
    let slots = slots();
    let count = |slot: &str| slots.iter().filter(|s| *s == slot).count();
    assert_eq!(count("ambre_lib::{impl#0}::call#0"), 2, "{slots:#?}");
    assert_eq!(count("ambre_lib::Toggle::toggle#0"), 1, "{slots:#?}");
    assert_eq!(count("ambre_lib::{impl#1}::toggle#0"), 1, "{slots:#?}");
    assert_eq!(count("ambre_lib::deep::set#0"), 1, "{slots:#?}");
}

// A call through a value names no `fn`: its literal reaches no slot, which a
// reviewer holds (`ARCH-041`).
#[test]
fn a_call_through_a_value_reaches_no_slot() {
    let slots = slots();
    assert_eq!(slots.len(), 12, "{slots:#?}");
}

// The slot of a parameter site is the one a call writes, so the judge joins
// the two.
#[test]
fn a_parameter_site_carries_the_slot_its_calls_write() {
    let ws = Workspace::new("slot");
    ws.file("crates/ambre-lib/src/lib.rs", LIB);
    let records = ws.compile("ambre_lib", "ambre-lib", &[], Build::Shipped);
    let slot = |key: &str| site(&records, key)[5].clone();
    assert_eq!(
        slot("ambre-lib | src/lib.rs | crate | set(on) | param | bool"),
        "ambre_lib::set#0"
    );
    assert_eq!(
        slot("ambre-lib | src/lib.rs | crate | Px::call(on) | param | bool"),
        "ambre_lib::{impl#0}::call#0"
    );
}

// A test build passes literals too: a test is a caller (`ARCH-040`).
#[test]
fn a_test_build_writes_its_literals() {
    let ws = Workspace::new("test-literals");
    ws.file(
        "crates/ambre-lib/src/lib.rs",
        "pub fn set(on: bool) {}\n#[cfg(test)]\nmod tests { #[test] fn t() { super::set(true); } }\n",
    );
    let records = ws.compile("ambre_lib", "ambre-lib", &[], Build::Test);
    assert_eq!(of(&records, "lit").len(), 1, "{records:#?}");
}

// A prop of a component reaches the parameter of the `fn` that holds the
// component's body, by name, as the Leptos `#[component]` macro writes it.
#[test]
fn a_component_prop_reaches_its_parameter_by_name() {
    let ws = Workspace::new("component");
    ws.file(
        "crates/ambre-lib/src/lib.rs",
        "pub struct ToggleProps { pub on: bool }\n\
         pub struct TogglePropsBuilder;\n\
         impl TogglePropsBuilder { pub fn on(self, on: bool) -> Self { self } }\n\
         pub fn Toggle(props: ToggleProps) {}\n\
         pub fn __component_toggle(on: bool) {}\n\
         pub fn view() { TogglePropsBuilder.on(true); }\n",
    );
    let records = ws.compile("ambre_lib", "ambre-lib", &[], Build::Shipped);
    let key = "ambre-lib | src/lib.rs | crate | Toggle(on) | param | bool";
    assert_eq!(site(&records, key)[5], "component:ambre_lib::Toggle:on");
    assert!(
        of(&records, "lit")
            .iter()
            .any(|l| l.starts_with("component:ambre_lib::Toggle:on\t")),
        "{records:#?}"
    );
}
