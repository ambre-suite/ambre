//! The browser-driven belt (`ambre-spec-testing.md` section 4).
//!
//! The wasm tier runs one crate's tests inside a page that the harness builds.
//! It boots no application: no Logic thread, no Sync worker, no canvas. This
//! belt serves the Trunk build of the whole application and opens it in a
//! headless Chrome through WebDriver. It drives it as a user does: with the
//! pointer, the keyboard, a file and a download.
//!
//! It reads what the user would read: the page, the live region a screen
//! reader hears, a toast, a downloaded file. It reaches no internal state, so
//! a test here holds what no test below the page can hold.
//!
//! Every test is behind the `browser` feature. `just test-browser` builds the
//! application, starts chromedriver and runs them.

#[cfg(test)]
mod busy;
#[cfg(test)]
mod driver;
#[cfg(test)]
mod frame;
#[cfg(test)]
mod ime;
#[cfg(test)]
mod page;
#[cfg(test)]
mod prose;
#[cfg(test)]
mod relay;
#[cfg(test)]
mod serve;
#[cfg(test)]
mod store;
#[cfg(test)]
mod toile;
#[cfg(test)]
mod trame;
