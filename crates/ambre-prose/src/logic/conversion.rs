//! The commands that wait on a converter module
//! (`ambre-spec-architecture.md section 2`, the converter modules).
//!
//! A Markdown import and a paste that carries HTML from another application
//! need the markup module. The Logic dispatcher asks
//! [`ProseState::conversion_family`] which module a command waits on, loads
//! it, then hands the command back through [`ProseState::handle_conversion`].
//! The module reads first. The command then runs the path a view command
//! runs: caught up, refused while suggesting where it must be, and named on a
//! refusal.

use ambre_bus::interop::InteropRequest;
use ambre_convert::markup::HtmlTree;
use ambre_convert::{Conversion, Converted, Converter};
use ambre_core::ConverterFamily;

use super::ProseState;
use super::clipboard;
use crate::events::ProseOutcome;
use crate::interop::{ProseIr, markdown_read};
use crate::message::{ConversionReport, InteropError, ProseCommand, ProseImport};

/// What a module read for the command in flight.
pub(super) enum ConvertedInput {
    /// The tree of a paste's HTML half, or none when the module could not
    /// read it.
    Html(Option<HtmlTree>),
    /// A Markdown file mapped onto the form, or why it was not.
    Markdown(Result<(ProseIr, ConversionReport), InteropError>),
}

impl ConvertedInput {
    pub(super) fn html(self) -> Option<HtmlTree> {
        match self {
            Self::Html(tree) => tree,
            Self::Markdown(_) => None,
        }
    }

    pub(super) fn markdown(self) -> Option<Result<(ProseIr, ConversionReport), InteropError>> {
        match self {
            Self::Markdown(read) => Some(read),
            Self::Html(_) => None,
        }
    }
}

impl ProseState {
    /// The converter family `command` waits on, if any: the markup module
    /// for a Markdown import, and for a paste whose richest readable half is
    /// HTML. A paste of this application's own bytes reads no HTML and waits
    /// on nothing.
    #[must_use]
    pub fn conversion_family(command: &ProseCommand) -> Option<ConverterFamily> {
        match command {
            ProseCommand::Paste {
                html: Some(_),
                payload,
                ..
            } if !clipboard::is_own(payload.as_ref()) => Some(ConverterFamily::Markup),
            ProseCommand::Interop(InteropRequest::Import(_)) => Some(ConverterFamily::Markup),
            _ => None,
        }
    }

    /// Run a view command [`Self::conversion_family`] names, once its module
    /// read what the command carries. `Err` carries the reason the module did
    /// not load. An import then fails with it. A paste falls through to its
    /// plain text, so the user loses the formatting and keeps the words.
    pub fn handle_conversion(
        &mut self,
        command: ProseCommand,
        converter: Result<&Converter, &str>,
    ) -> ProseOutcome {
        self.converted = match &command {
            ProseCommand::Paste {
                html: Some(html), ..
            } => Some(ConvertedInput::Html(converter.ok().and_then(|converter| {
                match converter.convert(&Conversion::ReadHtml { html: html.clone() }) {
                    Ok(Converted::Html(tree)) => Some(tree),
                    _ => None,
                }
            }))),
            ProseCommand::Interop(InteropRequest::Import(ProseImport { data, .. })) => {
                Some(ConvertedInput::Markdown(match converter {
                    Ok(converter) => markdown_read::read(data, converter),
                    Err(reason) => Err(InteropError::Stopped(reason.to_owned())),
                }))
            }
            _ => None,
        };
        let outcome = self.handle_view_command(command);
        self.converted = None;
        outcome
    }
}

#[cfg(test)]
impl ProseState {
    /// [`Self::handle_conversion`] through the markup engine the host links
    /// in place. A test drives a paste or an import as the Logic dispatcher
    /// does once the module loaded.
    pub(crate) fn handle_converted(&mut self, command: ProseCommand) -> ProseOutcome {
        self.handle_conversion(command, Ok(&host_converter()))
    }
}

/// The markup engine the host links in place: a test converts through the
/// request bytes a module reads.
#[cfg(test)]
pub(crate) fn host_converter() -> Converter {
    match Converter::load(
        ConverterFamily::Markup,
        &ambre_core::Payload::new(Vec::new()),
    ) {
        Ok(converter) => converter,
        Err(e) => panic!("the host links the markup engine: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ProseEvent;
    use crate::message::DocChar;
    use ambre_core::index::Pos;

    /// `ARCH-085`: a paste of Ambre's own bytes reads no HTML, so it waits on
    /// no module. The same HTML from another application waits on the markup
    /// module.
    #[test]
    fn a_paste_of_ambre_bytes_waits_on_no_module() {
        let mut state = ProseState::new().expect("a document");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "own".to_owned(),
        });
        let copied = state.handle_command(ProseCommand::CopyRange {
            anchor: (block, Pos::<DocChar>::ZERO),
            focus: (block, Pos::<DocChar>::new(3)),
        });
        let Some(ProseEvent::Clipboard { payload, html, .. }) = copied.display.into_iter().next()
        else {
            panic!("a copy answers with the clipboard");
        };
        let paste = |payload| ProseCommand::Paste {
            at: (block, Pos::<DocChar>::ZERO),
            text: "own".to_owned(),
            html: Some(html.clone()),
            payload,
        };
        assert_eq!(ProseState::conversion_family(&paste(Some(payload))), None);
        assert_eq!(
            ProseState::conversion_family(&paste(None)),
            Some(ConverterFamily::Markup)
        );
    }
}
