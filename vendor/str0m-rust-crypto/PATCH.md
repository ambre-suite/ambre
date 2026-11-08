# Patched copy of str0m-rust-crypto 0.7.0

Upstream: https://github.com/algesten/str0m (crate `str0m-rust-crypto` 0.7.0),
licensed MIT OR Apache-2.0 by Martin Algesten. The workspace uses this copy
through `[patch.crates-io]` in the root `Cargo.toml`.

The copy holds the published crate, less `.cargo_vcs_info.json`, `Cargo.lock`
and `Cargo.toml.orig`, which cargo writes at publication.

## The change

Each change against the published crate, read on 2026-11-08 by a `diff -r`
against the crate from crates.io:

* `Cargo.toml`:
  * dimpl takes the `rust-crypto` feature alone. Its `rcgen` feature enabled
    aws-lc-rs, and with it aws-lc-sys, a C library.
  * p256 adds the `ecdsa` and `pkcs8` features to `ecdh`, for the key of the
    certificate.
  * Two dependencies more: `x509-cert` 0.3 with the `std` and `builder`
    features and no default ones, and `rand` 0.10.
* `src/cert.rs`, a new file: a self-signed ECDSA P-256 certificate built with
  `x509-cert` and `p256`, as dimpl's rcgen path built it: a random 16-byte
  serial, one year of validity, the key as PKCS#8 DER.
* `src/lib.rs`: declares the module `cert`.
* `src/dtls.rs`: `generate_certificate` calls `src/cert.rs`.

str0m's own test suite passes on this copy (1 226 tests,
`--no-default-features --features rust-crypto`, read on 2026-11-07).

Drop this directory and the patch line once str0m ships the same change.
