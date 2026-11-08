//! The fixture tests of the driver: each compiles fixture crates through it,
//! as cargo would, and reads the records each unit writes
//! (`ambre-spec-testing.md` section 2).

#[cfg(test)]
mod harness;

#[cfg(test)]
mod bounds;
#[cfg(test)]
mod checks;
#[cfg(test)]
mod literals;
#[cfg(test)]
mod secrets;
#[cfg(test)]
mod sites;
#[cfg(test)]
mod unwrap;
