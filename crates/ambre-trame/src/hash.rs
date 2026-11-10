//! The hash tables of Trame: the tables of `ambre_core::hash`, under a key
//! drawn once per process from the platform entropy source (`TRAME-292`).
//! `clippy.toml` bans the std tables in this crate, so every table names
//! this module.

pub use ambre_core::hash::{HashMap, HashSet};
