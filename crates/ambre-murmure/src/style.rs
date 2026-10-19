//! The style marks of a message body (the Murmure composer spec).
//!
//! Bold, italic and inline code are marks that hold `true`. A link is a mark
//! that holds its address, `http` or `https` only (`MURMURE-561`). Each mark
//! sits on the body beside the `mention` marks (`MURMURE-562`). The marks are
//! the truth: nothing here parses the text.

use ambre_bus::{StyleMark, TextStyle};
use loro::{ExpandType, LoroDoc, LoroText, LoroValue, StyleConfig, StyleConfigMap, TextDelta};

use crate::error::MurmureError;
use crate::schema::{BOLD, CODE, ITALIC, LINK, MENTION};

/// Every key a style mark takes, in the order a read lists them.
const KEYS: [&str; 4] = [BOLD, ITALIC, CODE, LINK];

/// Set the expansion of every mark a body takes. The config is local to the
/// document and read at the mark: a receiver takes it from the op itself.
///
/// A mention and inline code expand to nothing, so text typed beside them
/// stays out. Bold and italic grow with text typed at their end.
pub(crate) fn configure(doc: &LoroDoc) {
    let mut styles = StyleConfigMap::default_rich_text_config();
    for key in [MENTION, CODE] {
        styles.insert(
            key.into(),
            StyleConfig {
                expand: ExpandType::None,
            },
        );
    }
    doc.config_text_style(styles);
}

/// Whether `address` is one a link may hold: `http` or `https`, with no
/// blank or control char in it (`MURMURE-561`).
pub fn linkable(address: &str) -> bool {
    let lower = address.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|rest| !rest.is_empty())
        && !address.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Whether every range of `styles` holds chars of `text`, every link holds
/// an address [`linkable`] takes, and no two links share a char.
pub(crate) fn fits(styles: &[StyleMark], text: &str) -> bool {
    let len = text.chars().count();
    let placed = styles
        .iter()
        .all(|mark| !mark.range.is_empty() && mark.range.end <= len);
    let links: Vec<_> = styles
        .iter()
        .filter_map(|mark| match &mark.style {
            TextStyle::Link(address) => Some((address, &mark.range)),
            _ => None,
        })
        .collect();
    let addressed = links.iter().all(|(address, _)| linkable(address));
    let apart = links.iter().enumerate().all(|(i, (_, a))| {
        links
            .iter()
            .skip(i + 1)
            .all(|(_, b)| a.end <= b.start || b.end <= a.start)
    });
    placed && addressed && apart
}

/// `styles` as a read gives them back: the runs of one style that touch or
/// overlap merged, in text order.
pub(crate) fn normalized(styles: &[StyleMark]) -> Vec<StyleMark> {
    let mut sorted = styles.to_vec();
    sorted.sort_by(|a, b| (&a.style, a.range.start).cmp(&(&b.style, b.range.start)));
    let mut merged: Vec<StyleMark> = Vec::with_capacity(sorted.len());
    for mark in sorted {
        match merged.last_mut() {
            Some(last) if last.style == mark.style && mark.range.start <= last.range.end => {
                last.range.end = last.range.end.max(mark.range.end);
            }
            _ => merged.push(mark),
        }
    }
    merged.sort_by(|a, b| (a.range.start, &a.style).cmp(&(b.range.start, &b.style)));
    merged
}

/// Mark each style of `styles` over `content`. The caller checked them with
/// [`fits`].
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a mark.
pub(crate) fn write(
    doc: &LoroDoc,
    content: &LoroText,
    styles: &[StyleMark],
) -> Result<(), MurmureError> {
    if styles.is_empty() {
        return Ok(());
    }
    configure(doc);
    for mark in styles {
        let value = match &mark.style {
            TextStyle::Link(address) => LoroValue::from(address.as_str()),
            TextStyle::Bold | TextStyle::Italic | TextStyle::Code => LoroValue::Bool(true),
        };
        content.mark(mark.range.clone(), key(&mark.style), value)?;
    }
    Ok(())
}

/// The key the mark of `style` sits under.
pub(crate) fn key(style: &TextStyle) -> &'static str {
    match style {
        TextStyle::Bold => BOLD,
        TextStyle::Italic => ITALIC,
        TextStyle::Code => CODE,
        TextStyle::Link(_) => LINK,
    }
}

/// Take every style mark off `content`, so a write from a draft leaves no
/// style the draft took out.
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses an unmark.
pub(crate) fn clear(content: &LoroText) -> Result<(), MurmureError> {
    let len = content.len_unicode();
    if len > 0 {
        for key in KEYS {
            content.unmark(0..len, key)?;
        }
    }
    Ok(())
}

/// The style a mark under `key` holding `value` gives, or `None` for a value
/// no style takes. A link to an address [`linkable`] refuses reads as text.
fn style_of(key: &str, value: &LoroValue) -> Option<TextStyle> {
    match (key, value) {
        (BOLD, LoroValue::Bool(true)) => Some(TextStyle::Bold),
        (ITALIC, LoroValue::Bool(true)) => Some(TextStyle::Italic),
        (CODE, LoroValue::Bool(true)) => Some(TextStyle::Code),
        (LINK, LoroValue::String(address)) if linkable(address) => {
            Some(TextStyle::Link(address.to_string()))
        }
        _ => None,
    }
}

/// The styles `content` carries, in text order, one per run. A read, never a
/// write.
pub(crate) fn read(content: &LoroText) -> Vec<StyleMark> {
    let mut styles: Vec<StyleMark> = Vec::new();
    // The run each key holds open, as an index into `styles`.
    let mut open: [Option<usize>; KEYS.len()] = [None; KEYS.len()];
    let mut at = 0;
    for span in content.to_delta() {
        let TextDelta::Insert { insert, attributes } = span else {
            continue;
        };
        let len = insert.chars().count();
        for (slot, key) in open.iter_mut().zip(KEYS) {
            let style = attributes
                .as_ref()
                .and_then(|attributes| attributes.get(key))
                .and_then(|value| style_of(key, value));
            let Some(style) = style else {
                *slot = None;
                continue;
            };
            match slot.and_then(|i| styles.get_mut(i)) {
                Some(run) if run.style == style && run.range.end == at => run.range.end += len,
                _ => {
                    *slot = Some(styles.len());
                    styles.push(StyleMark {
                        style,
                        range: at..at + len,
                    });
                }
            }
        }
        at += len;
    }
    styles.sort_by(|a, b| (a.range.start, &a.style).cmp(&(b.range.start, &b.style)));
    styles
}
