# Patched copy of leptos_macro 0.8.20

Upstream: https://github.com/leptos-rs/leptos (crate `leptos_macro` 0.8.20),
licensed MIT by Greg Johnston and the Leptos contributors. The workspace uses
this copy through `[patch.crates-io]` in the root `Cargo.toml`.

The copy holds `src/`, `Cargo.toml`, `build.rs` and `README.md` of the
published crate. The tests, `Makefile.toml`, `Cargo.lock` and
`Cargo.toml.orig` are left out.

## Why

Two release builds of one commit must give one client digest (`ZK-038` of
`specs/1-Foundations/ambre-spec-zero-knowledge.md`). The `view!` macro
collected the slots of a component in a `HashMap`, which the standard library
seeds at random in each process, and emitted one builder call per slot in the
order of that map. So each compile of `ambre-app` wrote the builder calls of a
component with two slots in another order, and the Main instance took other
bytes: three compiles of one tree gave three digests on 2026-11-14.

## The change

Each change against the published crate, read on 2026-11-14 by a `diff -r`
against the crate from crates.io.

* `Cargo.toml`: drops the `[[test]]` tables, which name the files left out.
  Nothing else differs.
* `src/view/mod.rs`, `src/view/component_builder.rs`,
  `src/view/slot_helper.rs`: the slot maps are a `BTreeMap` in place of a
  `HashMap`, and each is consumed by `into_iter` in place of `drain`. The
  builder calls follow the order of the slot names. A builder call sets one
  field, so their order changes no behaviour.

Drop the copy once a release of leptos emits the slots in a fixed order.
