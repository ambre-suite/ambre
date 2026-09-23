//! Pagination: the document laid out on measured pages
//! (`features/ambre-spec-prose-pagination.md`).
//!
//! Everything here runs in the Logic thread (`PROSE-010`) and only reads the
//! document (`PROSE-079`). Nothing it computes is written back, so a page
//! number, a break position and a measured height exist here alone
//! (`PROSE-033`, `PROSE-065`).
//!
//! * [`geometry`] -- the page box and the two bands cut out of it.
//! * [`fonts`] -- every family label resolved to a face the shaper holds.
//! * [`measure`] -- the effective style of one block, and the height its
//!   shaped lines take.
//! * [`place`] -- measured blocks laid onto pages, under the constraint order.
//! * [`pass`] -- the walk that measures the whole document, and the layout it
//!   assembles.
//! * [`reflow`] -- the layout held between edits, and the reflow that updates
//!   it in place.

pub mod fonts;
pub mod geometry;
pub mod measure;
pub mod pass;
pub mod place;
pub mod reflow;
pub mod run;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod footnote_pages;

#[cfg(test)]
mod belts {
    //! What holds the two rules no single call site can (`PROSE-107`,
    //! `PROSE-113`).
    //!
    //! Both are bans, and a ban is only as good as the reach of what enforces
    //! it. A test over one function proves nothing about the next one added
    //! beside it. So these read the module's own source instead, and they fail
    //! on the line that would break the rule wherever it lands.

    /// Every source file of this module, with its name.
    const SOURCES: [(&str, &str); 6] = [
        ("geometry.rs", include_str!("geometry.rs")),
        ("fonts.rs", include_str!("fonts.rs")),
        ("measure.rs", include_str!("measure.rs")),
        ("pass.rs", include_str!("pass.rs")),
        ("place.rs", include_str!("place.rs")),
        ("reflow.rs", include_str!("reflow.rs")),
    ];

    /// The lines of a source, tests and comments dropped.
    ///
    /// A ban is about what runs, and the tests below name the very tokens they
    /// forbid. Everything from the `mod tests` marker on is therefore cut.
    fn running_lines(source: &str) -> Vec<(usize, &str)> {
        let body = match source.find("\nmod tests {") {
            Some(at) => source.split_at(at).0,
            None => source,
        };
        body.lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim()))
            .filter(|(_, line)| !line.starts_with("//") && !line.is_empty())
            .collect()
    }

    /// Measuring reads the shipped faces and never the loaded collection
    /// (`PROSE-107`).
    ///
    /// `fonts.rs` is the one file allowed to touch a collection at all. It
    /// reads one to report a gap, never to choose a face. Anywhere else, a
    /// collection lookup is a measurement bent by the device.
    #[test]
    fn only_the_font_module_asks_a_context_what_it_holds() {
        for (name, source) in SOURCES {
            if name == "fonts.rs" {
                continue;
            }
            for (line, text) in running_lines(source) {
                assert!(
                    !text.contains(".collection"),
                    "{name}:{line} reaches into a font collection: {text}"
                );
            }
        }
    }

    /// Nothing that tells one peer from another reaches a measurement
    /// (`PROSE-113`).
    ///
    /// A layout is a pure function of the document, the page box and the
    /// shipped faces. A peer id, a clock or a device setting in here would
    /// make two peers disagree without either being wrong.
    #[test]
    fn nothing_identifying_a_peer_reaches_a_measurement() {
        const BANNED: [&str; 6] = [
            "PeerID",
            "peer_id",
            "SystemTime",
            "Instant",
            "now_ms",
            "random",
        ];
        for (name, source) in SOURCES {
            for (line, text) in running_lines(source) {
                for banned in BANNED {
                    assert!(
                        !text.contains(banned),
                        "{name}:{line} names `{banned}` in a measurement: {text}"
                    );
                }
            }
        }
    }
}
