//! The test oracle of the `Debug` form of document content.
//!
//! A type that holds text a user typed, pasted or imported prints the text
//! by its length. So a log line or a panic message carries no content
//! (`ambre-spec-zero-knowledge.md`). Each such type writes its `Debug` by
//! hand and hands its text to [`ambre_core::Redacted`]. This module holds
//! the check that each such `Debug` leaves the text out.

#[cfg(test)]
pub(crate) mod tests {
    /// Whether the `Debug` form of `value` leaves out `secret`, and names
    /// its length instead.
    pub(crate) fn hides(value: &impl std::fmt::Debug, secret: &str) -> bool {
        let printed = format!("{value:?}");
        // A delimiter, or the start of the form, before the count keeps
        // `6 chars` from matching `16 chars`.
        let length = format!("{} chars", secret.chars().count());
        !printed.contains(secret)
            && (printed.starts_with(&length)
                || printed.contains(&format!("({length}"))
                || printed.contains(&format!(" {length}")))
    }

    #[test]
    fn redacted_text_prints_its_length_alone() {
        let text = ambre_core::Redacted::new("secret");
        assert_eq!(format!("{text:?}"), "6 chars");
        assert!(hides(&text, "secret"));
    }

    #[test]
    fn an_input_prints_its_text_by_length_alone() {
        use crate::input::ProseInput;
        assert!(hides(&ProseInput::Text("typed".into()), "typed"));
        assert!(hides(&ProseInput::Compose("composed".into()), "composed"));
        assert!(hides(
            &ProseInput::ComposeEnd("committed".into()),
            "committed"
        ));
        let paste = ProseInput::Paste {
            text: "pasted".into(),
            html: Some("<b>marked</b>".into()),
            rich: None,
        };
        assert!(hides(&paste, "pasted") && hides(&paste, "<b>marked</b>"));
    }

    #[test]
    fn a_run_of_the_form_prints_its_text_by_length_alone() {
        assert!(hides(&crate::interop::IrRun::plain("imported"), "imported"));
    }

    #[test]
    fn a_block_of_the_form_prints_its_text_by_length_alone() {
        use crate::interop::IrBlock;
        let code = IrBlock::CodeBlock {
            lang: ambre_core::Validated::<crate::message::CodeLang>::parse("privatelang").ok(),
            text: "let secret = 1;".into(),
        };
        assert!(hides(&code, "let secret = 1;"));
        // The language is a word the user typed after a fence.
        assert!(hides(&code, "privatelang"));
        let image = IrBlock::Image(Box::new(ambre_prose_form::IrImage {
            alt: "a private photo".into(),
            caption: "the private caption".into(),
            ..ambre_prose_form::IrImage::default()
        }));
        assert!(hides(&image, "a private photo"));
        assert!(hides(&image, "the private caption"));
    }

    #[test]
    fn the_meta_of_the_form_prints_its_title_by_length_alone() {
        let meta = crate::interop::IrMeta {
            title: Some("Quarterly plan".into()),
            ..crate::interop::IrMeta::default()
        };
        assert!(hides(&meta, "Quarterly plan"));
    }

    #[test]
    fn a_reply_intent_prints_its_text_by_length_alone() {
        let reply = crate::view::annotate::AnnotationIntent::Reply {
            comment_id: ambre_core::Id::mint(),
            text: "my reply".into(),
        };
        assert!(hides(&reply, "my reply"));
    }
}
