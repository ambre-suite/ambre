// ambre-chrome/src/hover_scan.rs
// ============================================================================
// The scan of the shell stylesheet for a control a hover alone reveals
// (`CHROME-183`, `ambre-spec-shell.md` section 2).
//
// A hover rule reveals a control when a base rule hides the same target and
// the hover rule shows it again. Each such rule needs two twins: one under
// the keyboard focus, and one under `@media (hover: none)`, where a touch
// screen reads it. A pseudo-element is a tooltip, not a control, so the scan
// skips it.
// ============================================================================

/// The shell stylesheet, as the page loads it.
const STYLES: &str = include_str!("../../ambre-app/styles.css");

/// One rule: its selectors, its declarations, and the `@media` condition
/// around it, empty at the top level.
struct Rule<'a> {
    media: &'a str,
    selectors: Vec<String>,
    body: String,
}

/// The sheet without its comments.
fn uncommented(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find("*/")
            .map_or("", |end| &rest[start + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// One selector with its spaces collapsed.
fn normal(selector: &str) -> String {
    selector.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every rule of `css`, one `@media` level deep. Another at-rule, such as
/// `@keyframes`, holds no selector of a control, and is skipped whole.
fn rules(css: &str) -> Vec<Rule<'_>> {
    let mut out = Vec::new();
    let mut rest = css;
    let mut media = "";
    while let Some(open) = rest.find(['{', '}']) {
        let head = rest[..open].trim();
        if rest[open..].starts_with('}') {
            // The end of the `@media` block in force.
            media = "";
            rest = &rest[open + 1..];
            continue;
        }
        if let Some(condition) = head.strip_prefix("@media") {
            media = condition.trim();
            rest = &rest[open + 1..];
            continue;
        }
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let body = &rest[open + 1..open + close];
        if head.starts_with('@') {
            // A nested block: skip to the brace that closes the at-rule.
            let mut depth = 0_i32;
            let mut end = rest.len();
            for (at, c) in rest[open..].char_indices() {
                depth += match c {
                    '{' => 1,
                    '}' => -1,
                    _ => 0,
                };
                if depth == 0 {
                    end = open + at + 1;
                    break;
                }
            }
            rest = &rest[end..];
            continue;
        }
        out.push(Rule {
            media,
            selectors: head.split(',').map(normal).collect(),
            body: body.split_whitespace().collect::<Vec<_>>().join(" "),
        });
        rest = &rest[open + close + 1..];
    }
    out
}

/// Whether a declaration block hides what it styles.
fn hides(body: &str) -> bool {
    let body = format!("{};", body.trim_end_matches(';'));
    ["opacity: 0;", "visibility: hidden;", "display: none;"]
        .iter()
        .any(|hidden| body.contains(hidden))
}

/// Whether a declaration block shows what a base rule hid: an opacity, a
/// visibility or a display that is no hiding one. A colour alone reveals
/// nothing.
fn reveals(body: &str) -> bool {
    !hides(body)
        && ["opacity:", "visibility: visible", "display:"]
            .iter()
            .any(|shown| body.contains(shown))
}

/// Each hover selector of `css` that reveals a control with no twin under
/// the keyboard focus, or none under `@media (hover: none)`.
fn hover_only(css: &str) -> Vec<String> {
    let css = uncommented(css);
    let all = rules(&css);
    let hidden = |target: &str| {
        all.iter().any(|r| {
            r.media.is_empty() && hides(&r.body) && r.selectors.iter().any(|s| s == target)
        })
    };
    let shown_by = |selector: &str| {
        all.iter().any(|r| {
            r.media.is_empty() && reveals(&r.body) && r.selectors.iter().any(|s| s == selector)
        })
    };
    let touch_shows = |target: &str| {
        all.iter().any(|r| {
            r.media.contains("hover: none")
                && reveals(&r.body)
                && r.selectors.iter().any(|s| s == target)
        })
    };
    all.iter()
        .filter(|r| r.media.is_empty() && reveals(&r.body))
        .flat_map(|r| r.selectors.iter())
        .filter(|s| s.contains(":hover") && !s.contains("::"))
        .filter_map(|selector| {
            let target = normal(&selector.replace(":hover", ""));
            if !hidden(&target) {
                return None;
            }
            let keyboard = [":focus-within", ":focus-visible", ":focus"]
                .iter()
                .any(|focus| shown_by(&selector.replace(":hover", focus)));
            let touch = touch_shows(&target);
            (!(keyboard && touch)).then(|| selector.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CHROME-183`: no rule of the shell stylesheet reveals a control by
    /// hover alone. Each one the scan finds shows under the keyboard focus
    /// and on a touch screen too.
    #[test]
    fn no_rule_of_the_shell_reveals_a_control_by_hover_alone() {
        assert_eq!(hover_only(STYLES), Vec::<String>::new());
        // The launcher's actions are a hover reveal the scan reads.
        let css = uncommented(STYLES);
        assert!(rules(&css).iter().any(|r| {
            r.selectors
                .iter()
                .any(|s| s == ".ambre-doc-item:hover .ambre-doc-actions")
        }));
    }

    /// The scan goes red on each missing twin, and passes a tooltip.
    #[test]
    fn the_scan_names_a_reveal_that_lacks_a_twin() {
        let base = ".row .act { opacity: 0; }\n.row:hover .act { opacity: 1; }\n";
        assert_eq!(hover_only(base), [".row:hover .act"]);
        let keyboard = format!("{base}.row:focus-within .act {{ opacity: 1; }}\n");
        assert_eq!(hover_only(&keyboard), [".row:hover .act"]);
        let both =
            format!("{keyboard}@media (hover: none) {{\n  .row .act {{ opacity: 1; }}\n}}\n");
        assert_eq!(hover_only(&both), Vec::<String>::new());
        let touch_alone = format!("{base}@media (hover: none) {{ .row .act {{ opacity: 1; }} }}\n");
        assert_eq!(hover_only(&touch_alone), [".row:hover .act"]);
        let tooltip = ".tip::after { opacity: 0; }\n.tip:hover::after { opacity: 1; }\n";
        assert_eq!(hover_only(tooltip), Vec::<String>::new());
        let colour = ".edge { display: none; }\n.edge:hover { background: red; }\n";
        assert_eq!(
            hover_only(colour),
            Vec::<String>::new(),
            "a colour reveals nothing"
        );
    }
}
