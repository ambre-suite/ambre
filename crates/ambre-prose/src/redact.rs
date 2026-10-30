//! The `Debug` form of document content: its length alone.
//!
//! A type that holds text a user typed, pasted or imported prints the text
//! by its length. So a log line or a panic message carries no content
//! (`ambre-spec-zero-knowledge.md`). Each such type writes its `Debug` by
//! hand and hands its text to [`Redacted`].

use std::fmt;

/// Text that a `Debug` impl prints by its length in bytes alone.
pub(crate) struct Redacted<'a>(pub(crate) &'a str);

impl fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} bytes>", self.0.len())
    }
}

/// An optional text, printed by its length alone when present.
pub(crate) fn optional(text: Option<&String>) -> Option<Redacted<'_>> {
    text.map(|text| Redacted(text))
}

#[cfg(test)]
pub(crate) mod tests {
    /// Whether the `Debug` form of `value` leaves out `secret`, and names
    /// its length instead.
    pub(crate) fn hides(value: &impl std::fmt::Debug, secret: &str) -> bool {
        let printed = format!("{value:?}");
        !printed.contains(secret) && printed.contains(&format!("<{} bytes>", secret.len()))
    }

    #[test]
    fn redacted_text_prints_its_length_alone() {
        assert_eq!(format!("{:?}", super::Redacted("secret")), "<6 bytes>");
        assert!(hides(&super::Redacted("secret"), "secret"));
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
            lang: ambre_bus::app::prose::CodeLang::new("rust"),
            text: "let secret = 1;".into(),
        };
        assert!(hides(&code, "let secret = 1;"));
        let image = IrBlock::Image {
            alt: "a private photo".into(),
        };
        assert!(hides(&image, "a private photo"));
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
            comment_id: ambre_core::CommentId::new(),
            text: "my reply".into(),
        };
        assert!(hides(&reply, "my reply"));
    }
}
