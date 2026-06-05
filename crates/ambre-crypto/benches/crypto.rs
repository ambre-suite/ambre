//! Criterion benchmarks for the crypto envelope.
//!
//! Targets (desktop; mobile measured later) from ambre-spec-sync.md §5:
//!   seal 1 KB  < 200 us (P95), open 1 KB < 350 us (P95).

use ambre_crypto::keys::{DocumentKey, MasterSeed};
use ambre_crypto::{envelope, passphrase};
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

fn bench_envelope(c: &mut Criterion) {
    let doc_key = DocumentKey::from_array([0x11; 32]);
    let signing = MasterSeed::from_array([0x22; 32]).derive_signing_key();
    let author = signing.verifying_key().to_bytes();
    let aad = b"public-data-metadata";
    let payload = vec![0x42u8; 1024];

    c.bench_function("seal_1kb", |b| {
        b.iter(|| envelope::seal(black_box(&payload), black_box(aad), &doc_key, &signing));
    });

    let sealed = envelope::seal(&payload, aad, &doc_key, &signing);
    c.bench_function("open_1kb", |b| {
        b.iter(|| envelope::open(black_box(&sealed), black_box(aad), &doc_key, &author).unwrap());
    });
}

fn bench_argon2(c: &mut Criterion) {
    let salt = [0xABu8; 16];
    c.bench_function("argon2id_derive", |b| {
        b.iter(|| passphrase::derive_user_key(black_box(b"correct horse"), black_box(&salt)).unwrap());
    });
}

criterion_group!(benches, bench_envelope, bench_argon2);
criterion_main!(benches);
