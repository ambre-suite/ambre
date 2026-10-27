// crates/ambre-canvas/src/bin/ambre-tokens.rs
// ============================================================================
// The design-token driver (`ambre-spec-design-tokens.md`). Two modes, one per
// recipe:
//
//   * no argument: `just tokens-check` compares the committed theme files with
//     what `tokens.ron` generates, and fails on any difference;
//   * `--write`: `just tokens-write` rewrites both files.
//
// A binary rather than a test, because the rewrite writes the working copy and
// a test harness must not. `required-features` keeps it out of every other
// build, the wasm one included. The generator itself is `ambre_canvas::tokens`.
// ============================================================================

use std::process::ExitCode;

use ambre_canvas::tokens::{self, SOURCE};

/// What one run does.
enum Mode {
    /// Compare the committed files with the source, and write nothing.
    Check,
    /// Rewrite the committed files from the source.
    Write,
}

fn main() -> ExitCode {
    let mode = match std::env::args().nth(1).as_deref() {
        None => Mode::Check,
        Some("--write") => Mode::Write,
        Some(_) => {
            eprintln!("usage: ambre-tokens [--write]");
            return ExitCode::FAILURE;
        }
    };
    let generated = match tokens::generate_from_disk() {
        Ok(generated) => generated,
        Err(e) => {
            eprintln!("tokens: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Mode::Write = mode {
        return match tokens::write(&generated) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("tokens: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let stale = tokens::stale(&generated);
    if stale.is_empty() {
        return ExitCode::SUCCESS;
    }
    for path in stale {
        eprintln!("tokens: crates/ambre-canvas/{path} is not what {SOURCE} generates");
    }
    eprintln!("tokens: run `just tokens-write` and commit the result");
    ExitCode::FAILURE
}
