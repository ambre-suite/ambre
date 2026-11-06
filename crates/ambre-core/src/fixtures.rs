//! Document-fixture types -- the generator contract and the sidecar manifest
//! (`ambre-spec-fixtures.md`).
//!
//! **Why here.** Implementors live next to the engine whose commands they drive
//! (`ambre-trame/src/fixtures/`, `ambre-toile/src/fixtures/`), so a schema change
//! and its generator move in one commit. Rust's orphan rule then forbids defining
//! [`FixtureGen`] anywhere downstream of them: `ambre-logic` depends on the apps,
//! never the reverse. `ambre-core` is upstream of everything, already owns
//! [`DocType`], and already carries `serde` and `hex`. So the trait and the
//! artifact types go here, and the *behaviour* (loader, driver) stays in
//! `ambre-logic`.
//!
//! Everything in this module is fixture code (I4) and sits behind the non-default
//! `fixtures` feature. No client build names it, so none of it can reach a wasm
//! artifact.

use crate::id::{Document, Entity, Id};
use crate::index::Len;
use crate::{Cells, Char, Count, DocType, Elements, Version};
use serde::{Deserialize, Serialize};

// -----------------------------------------------------------------------------
// Generator identity
// -----------------------------------------------------------------------------

/// The rule of the stable name of one generator, for example
/// `"trame/formula_dag_deep"`. A `Validated<FixtureName>` holds one.
///
/// The name is also the artifact stem, so it forms a file path. The rule
/// admits two parts joined by one `/`: the app, then a `snake_case` stem.
/// Each part starts with a lowercase ASCII letter, and holds only lowercase
/// ASCII letters, digits and `_`. So no name holds `..`, a leading `/`, an
/// empty part, an upper-case letter or another separator.
///
/// A generator names itself through [`FixtureGen::name`], a manifest through
/// its decode, and the corpus tooling through each stem it reads off disk. The
/// serde form is the bare string.
pub enum FixtureName {}

/// A text that is no fixture name. The error, `Refused<FixtureName>`, echoes
/// the refused text.
impl crate::Refuses for FixtureName {
    type Raw = String;
    const NAME: &'static str = "InvalidFixtureName";

    fn message(raw: &Self::Raw, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{raw:?} is not a fixture name: two lowercase snake_case parts, <app>/<stem>"
        )
    }
}

impl crate::Rule for FixtureName {
    type Error = crate::Refused<FixtureName>;

    const LOGGED_AS: Option<&'static str> = Some("FixtureName");

    fn admit(text: String) -> Result<String, crate::Refused<FixtureName>> {
        let part = |part: &str| {
            part.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        };
        let mut parts = text.split('/');
        let admitted = match (parts.next(), parts.next(), parts.next()) {
            (Some(app), Some(stem), None) => part(app) && part(stem),
            _ => false,
        };
        if admitted {
            Ok(text)
        } else {
            Err(crate::Refused::new(text))
        }
    }
}

/// A fixture name is a public identifier the tooling prints in a report.
impl crate::Displayed for FixtureName {}

/// The constant a generator author picks once. It pins the `PeerID`, the
/// commit clock and the `document_id` (I1). Transparent to serde.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy, Debug, Hash)]
#[serde(transparent)]
pub struct FixtureSeed(u64);

impl FixtureSeed {
    /// The seed `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The seed itself, for the mixer.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The peer id the seed pins: the seed itself, so a generator writes as
    /// the same peer on every run.
    #[must_use]
    pub const fn peer(self) -> crate::PeerId {
        crate::PeerId::new(self.0)
    }
}

/// The space of the version a generator author bumps when the produced
/// document changes on purpose. Transparent to serde.
pub enum Generator {}

impl crate::version::VersionKind for Generator {
    type Repr = u32;
    const NAME: &'static str = "GeneratorVersion";
}

impl crate::version::OpenRange for Generator {}

/// How many distinct peers wrote ops into a fixture.
pub enum Peers {}

impl crate::CountKind for Peers {
    type Repr = u32;
    const NAME: &'static str = "PeerCount";
}

/// How many sheets a workbook fixture holds.
pub enum Sheets {}

impl crate::CountKind for Sheets {
    type Repr = u32;
    const NAME: &'static str = "SheetCount";
}

/// How many stored formula sources a workbook fixture holds.
pub enum Formulas {}

impl crate::CountKind for Formulas {
    type Repr = u32;
    const NAME: &'static str = "FormulaCount";
}

/// How many frame elements a board fixture holds.
pub enum BoardFrames {}

impl crate::CountKind for BoardFrames {
    type Repr = u32;
    const NAME: &'static str = "BoardFrameCount";
}

/// How many connectors a board fixture holds.
pub enum Connectors {}

impl crate::CountKind for Connectors {
    type Repr = u32;
    const NAME: &'static str = "ConnectorCount";
}

/// How many blocks a document fixture holds, in document order.
pub enum DocBlocks {}

impl crate::CountKind for DocBlocks {
    type Repr = u32;
    const NAME: &'static str = "DocBlockCount";
}

// -----------------------------------------------------------------------------
// Seed -> document identity
// -----------------------------------------------------------------------------

/// The `document_id` a generator writes into its `.ambre` header, derived from
/// its seed.
///
/// The third entropy source of I1: `export_ambre_plaintext` takes a UUID v7,
/// which is time-ordered and partly random. A generator that minted one would
/// produce different bytes on every run. This derives all sixteen bytes from
/// `seed` instead, then stamps the version / variant nibbles. The result is still
/// a well-formed v7 that the container and the registry accept. Its timestamp
/// field simply carries seed material rather than a clock.
///
/// Crypto-free by design: the mixer is SplitMix64, a finalizer chosen for being
/// short and dependency-free. It is **not** a hash and nothing here relies on it
/// being one -- a fixture's identity is public by construction.
pub fn seeded_document_id(seed: FixtureSeed) -> Id<Document> {
    seeded_id(seed)
}

/// The id of any kind that a generator derives from its seed, as
/// [`seeded_document_id`] does. A fixture author takes the same bytes on
/// every run.
pub fn seeded_id<E: Entity<Repr = crate::uuid::Uuid>>(seed: FixtureSeed) -> Id<E> {
    let mut state = seed.get();
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        // SplitMix64: advance, then finalize. Two draws fill the sixteen bytes.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        chunk.copy_from_slice(&z.to_be_bytes());
    }
    // Version 7 in the high nibble of byte 6, RFC 4122 variant in the top two
    // bits of byte 8. Those are the two fields a reader may check.
    bytes[6] = (bytes[6] & 0x0F) | 0x70;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    Id::from_bytes(bytes)
}

// -----------------------------------------------------------------------------
// The importing peer
// -----------------------------------------------------------------------------

/// The peer id a fixture loader pins on the state it imports into.
///
/// An import needs a live state to receive it. That state may write under its
/// own peer: a Prose reader drops the root paragraph an older document does
/// not carry. Such a fixture then carries one peer that belongs to
/// the reader, not to the document. A random
/// peer there would also make the restored state differ run to run, which the
/// frozen comparison of `FIXTURE-019` reads back. A pinned value keeps the load
/// deterministic and lets [`fixture_peer_count`] subtract one known id. The
/// value is the loader's alone: a generator seeds its peer from its own seed
/// and must never pick this one.
pub const FIXTURE_IMPORT_PEER: crate::PeerId = crate::PeerId::new(0x10AD_0000_0000_0001);

/// Counts the *document's* own peers in `peers`, the oplog peer set.
/// [`FIXTURE_IMPORT_PEER`] is the reader, not an author, so it does not count.
/// Nor does `scaffold`, the identity every peer writes the scaffolding under
/// ([`scaffold_peer`](crate::schema::scaffold_peer)).
///
/// Takes [`PeerId`](crate::PeerId)s rather than a CRDT type: `ambre-core` is
/// upstream of the engines and does not depend on the CRDT library.
#[must_use]
pub fn fixture_peer_count(
    peers: impl IntoIterator<Item = crate::PeerId>,
    scaffold: crate::PeerId,
) -> Count<Peers> {
    Count::<Peers>::of(
        peers
            .into_iter()
            .filter(|peer| *peer != FIXTURE_IMPORT_PEER && *peer != scaffold)
            .count(),
    )
}

// -----------------------------------------------------------------------------
// The generator contract
// -----------------------------------------------------------------------------

/// One parametric document generator.
///
/// Method-based rather than all-associated-item. The driver iterates a `&[&dyn
/// FixtureGen]` registry, and a trait of bare consts and free functions is not
/// object-safe. It would force a hand-maintained table of function pointers,
/// one more place to forget a generator.
pub trait FixtureGen {
    /// Stable identity: the manifest's [`FixtureManifest::generator`] and the
    /// artifact **stem** -- the whole filename in `current/`, the part before the
    /// `@` in `frozen/`.
    ///
    /// The generator parses its own literal, so the rule of [`FixtureName`]
    /// runs at this door too, and no generator unwraps.
    ///
    /// # Errors
    ///
    /// The refused literal, when it breaks the rule of [`FixtureName`]. The
    /// driver then fails the run with that reason.
    fn name(&self) -> Result<crate::Validated<FixtureName>, crate::Refused<FixtureName>>;

    /// Which app's engine this generator drives, and therefore which importer
    /// reads the artifact back.
    fn doc_type(&self) -> DocType;

    /// A constant the generator author picks once. It belongs to the generator
    /// rather than the driver because a seed supplied from outside could differ
    /// between runs, turning the drift gate into noise. Changing it is a content
    /// change and takes a [`generator_version`](Self::generator_version) bump
    /// with it.
    fn seed(&self) -> FixtureSeed;

    /// Bumped by the author when the produced document changes on purpose; the
    /// driver copies it into the manifest, having no other source for it. No
    /// recipe reads it. It exists for the reviewer reading the manifest diff, and
    /// for the frozen belt, which retires its semantic comparison when this no
    /// longer matches.
    fn generator_version(&self) -> Version<Generator>;

    /// Build the document and return the plaintext `.ambre` container, that is,
    /// the output of the app's `export_ambre_plaintext()`. Not a raw snapshot: the
    /// header makes the artifact self-describing and loadable through the
    /// production importer.
    ///
    /// Implemented by driving `handle_command` on the app state (I2). [`seed`](Self::seed)
    /// pins the `PeerID`, the commit clock and the `document_id` (I1).
    ///
    /// Infallible: a generator drives commands it chose itself, so a rejected one
    /// is a bug, not a runtime condition. It **panics** rather than emit a
    /// half-built document -- a corpus that silently lost a merge is worse than a
    /// failed `just fixtures`.
    fn build(&self) -> crate::Payload<crate::PlaintextContainer>;
}

// -----------------------------------------------------------------------------
// The sidecar manifest
// -----------------------------------------------------------------------------

/// Provenance for one committed artifact, written next to it as JSON.
///
/// The `.ambre` header carries `doc_type` and `document_id` but no provenance,
/// which a drift gate needs. JSON rather than a binary encoding, because the
/// manifest exists *because* an opaque binary has no reviewable diff. It lets CI
/// and a human reviewer tell "regenerated, identical meaning" apart from "the
/// schema drifted".
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct FixtureManifest {
    /// Generator identity, for example `"trame/formula_dag_deep"`. Also the artifact
    /// **stem**: the whole filename in `current/`, the part before the `@` in
    /// `frozen/`. The decode runs the rule of [`FixtureName`], so a manifest
    /// that names a bad generator is refused on read.
    pub generator: crate::Validated<FixtureName>,

    /// Bumped by the generator author when the produced document changes on
    /// purpose. Frozen into the artifact too: that is how detects a frozen
    /// fixture and the current one are no longer comparable.
    pub generator_version: Version<Generator>,

    /// Declared by the generator ([`FixtureGen::seed`]), and pins the `PeerID`
    /// and `document_id` (I1).
    pub seed: FixtureSeed,

    pub doc_type: DocType,

    /// Container version (`docfile`) at write time -- current for a
    /// generated artifact, superseded for a frozen one.
    pub ambre_version: crate::Version<crate::Container>,

    /// Payload schema version (`ambre-spec-schema-version.md`). Distinct from
    /// [`generator_version`](Self::generator_version), which tracks authorial
    /// intent rather than format. Read back from the generated document rather
    /// than declared, so a manifest cannot name a version its artefact does not
    /// hold. `0` is what a document written before the register existed reads as
    /// (`SCHEMA-005`).
    pub schema_version: crate::Version<crate::Schema>,

    /// BLAKE2b over the **whole `.ambre` file, header included**, so a flipped
    /// `document_id` byte is caught by the write-once check. The digest
    /// is `hash::file_digest`, keyless and 32 bytes. Hex, not 32
    /// raw integers, which would defeat the readable diff this sidecar exists
    /// for.
    pub digest: crate::FixedBytes<crate::FileDigest>,

    pub shape: FixtureShape,
}

/// Coarse size/complexity summary. When the bytes move, this says whether the
/// document grew, lost a sheet, or is unchanged in shape. It is checked
/// against `shape_of` to catch a manifest committed out of sync with its
/// artifact. **Not** a selection index -- lookup is by name.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct FixtureShape {
    /// Number of distinct peers whose ops are present (1 for a solo document).
    pub peers: Count<Peers>,
    pub body: ShapeBody,
}

/// Per-app rather than a union of nullable counters. An application that joins
/// later adds a variant, and widens no existing row. `Prose` names blocks and
/// characters, and no row of `Trame` or `Toile` changed when it arrived.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub enum ShapeBody {
    Trame {
        cells: Count<Cells>,
        sheets: Count<Sheets>,
        formulas: Count<Formulas>,
    },
    Toile {
        elements: Count<Elements>,
        frames: Count<BoardFrames>,
        connectors: Count<Connectors>,
    },
    Prose {
        blocks: Count<DocBlocks>,
        characters: Len<Char>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Version;

    fn sample() -> FixtureManifest {
        FixtureManifest {
            generator: crate::Validated::parse("trame/formula_dag_deep").unwrap(),
            generator_version: Version::<Generator>::new(3),
            seed: FixtureSeed::new(0x0123_4567_89AB_CDEF),
            doc_type: DocType::Trame,
            ambre_version: crate::AMBRE_VERSION,
            schema_version: crate::SCHEMA_VERSION_ABSENT,
            digest: crate::FixedBytes::from_array([0xAB; 32]),
            shape: FixtureShape {
                peers: Count::<Peers>::new(2),
                body: ShapeBody::Trame {
                    cells: Count::<Cells>::new(12),
                    sheets: Count::<Sheets>::new(2),
                    formulas: Count::<Formulas>::new(4),
                },
            },
        }
    }

    #[test]
    fn the_manifest_round_trips_through_json() {
        let json = serde_json::to_string_pretty(&sample()).expect("serializes");
        let back: FixtureManifest = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, sample());
    }

    #[test]
    fn the_typed_fields_keep_the_bare_json_of_the_manifest() {
        // The names, seeds, versions and counts are newtypes. A committed
        // manifest is a frozen artifact, so the sidecar must read as it did
        // when they were bare.
        let json = serde_json::to_string(&sample()).expect("serializes");
        assert!(json.starts_with(concat!(
            "{\"generator\":\"trame/formula_dag_deep\",",
            "\"generator_version\":3,",
            "\"seed\":81985529216486895,"
        )));
        assert!(json.ends_with(concat!(
            "\"shape\":{\"peers\":2,",
            "\"body\":{\"Trame\":{\"cells\":12,\"sheets\":2,\"formulas\":4}}}}"
        )));
    }

    #[test]
    fn the_digest_is_hex_not_thirty_two_integers() {
        // E1: the sidecar is JSON *for the diff*. A raw byte array would encode
        // as `[171, 171, ...]` and defeat the only reason it is not binary.
        let json = serde_json::to_string(&sample()).expect("serializes");
        assert!(
            json.contains(&format!("\"digest\":\"{}\"", "ab".repeat(32))),
            "digest must be a hex string, got: {json}"
        );
    }

    #[test]
    fn a_bad_digest_length_is_a_deserialization_error_not_a_truncation() {
        let mut value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&sample()).unwrap()).unwrap();
        value["digest"] = serde_json::Value::String("abcd".to_owned());
        assert!(serde_json::from_value::<FixtureManifest>(value).is_err());
    }

    #[test]
    fn the_fixture_name_rule_admits_the_app_and_a_snake_case_stem() {
        for name in [
            "trame/formula_dag_deep",
            "toile/bench_board_50k",
            "prose/styled_document",
            "trame/gate_alpha",
        ] {
            assert!(
                crate::Validated::<FixtureName>::parse(name).is_ok(),
                "{name} is a fixture name"
            );
        }
    }

    #[test]
    fn the_fixture_name_rule_refuses_every_path_that_escapes_or_breaks_the_grammar() {
        for name in [
            "",
            "trame",
            "../trame/grid",
            "trame/../grid",
            "trame/..",
            "/trame/grid",
            "trame/grid/",
            "/etc/passwd",
            "trame//grid",
            "trame/",
            "/grid",
            "trame/a/b",
            "Trame/grid",
            "trame/Grid",
            "trame\\grid",
            "trame/grid-literals",
            "trame/grid.literals",
            "trame/grid literals",
            "trame/_grid",
            "trame/1grid",
        ] {
            let refused = crate::Validated::<FixtureName>::parse(name);
            assert!(refused.is_err(), "{name:?} is no fixture name");
            if let Err(error) = refused {
                assert_eq!(error.as_str(), name, "the error echoes the refused text");
            }
        }
    }

    #[test]
    fn a_manifest_that_names_a_bad_generator_is_refused_on_read() {
        let mut value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&sample()).unwrap()).unwrap();
        value["generator"] = serde_json::Value::String("../../etc/passwd".to_owned());
        assert!(serde_json::from_value::<FixtureManifest>(value).is_err());
    }

    #[test]
    fn a_seeded_document_id_is_stable_distinct_and_well_formed() {
        // I1's third entropy source: same seed, same id -- or the drift gate
        // fires on the header alone, whatever the payload does.
        let id = |seed| seeded_document_id(FixtureSeed::new(seed));
        assert_eq!(id(7), id(7));
        assert_ne!(id(7), id(8));
        // Two neighbouring seeds must not collide in the version/variant stamp
        // either: the nibbles are fixed, the other 122 bits are not.
        for seed in 0..64u64 {
            let bytes = *id(seed).as_bytes();
            assert_eq!(bytes[6] >> 4, 7, "version nibble for seed {seed}");
            assert_eq!(bytes[8] >> 6, 0b10, "variant bits for seed {seed}");
        }
        let all: std::collections::HashSet<[u8; 16]> =
            (0..256u64).map(|s| *id(s).as_bytes()).collect();
        assert_eq!(all.len(), 256, "seeds must not collide");
    }

    #[test]
    fn a_seeded_id_of_another_kind_holds_the_bytes_of_the_document_id() {
        let seed = FixtureSeed::new(0x5EED);
        let user: Id<crate::id::User> = seeded_id(seed);
        assert_eq!(user.as_bytes(), seeded_document_id(seed).as_bytes());
    }

    #[test]
    fn the_shape_body_is_tagged_by_app() {
        // The Toile variant carries no Trame field and vice versa: a later app
        // adds a variant rather than widening every row.
        let toile = ShapeBody::Toile {
            elements: Count::<Elements>::new(5),
            frames: Count::<BoardFrames>::new(1),
            connectors: Count::<Connectors>::new(2),
        };
        let json = serde_json::to_string(&toile).expect("serializes");
        assert!(json.contains("Toile"), "{json}");
        assert!(!json.contains("cells"), "{json}");
    }
}
