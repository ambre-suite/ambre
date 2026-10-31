// ambre-app/src/host/probe.rs
// ============================================================================
// The keystroke latency probes on the browser's timeline
// (`1-Foundations/ambre-spec-budgets.md` section 1).
//
// `ambre_prose::probe` decides what a measure is. This file stamps each one
// with `performance.measure`, so the profiler shows it beside the frames. The
// stamps are always on: one costs microseconds, and a slow session can then
// be measured where it happens. With `?probes` in the URL, the samples are
// also kept. Then every few seconds the console prints the median and the 95th
// percentile of each measure.
// ============================================================================

use std::collections::BTreeMap;

use wasm_bindgen::{JsCast, JsValue};

use ambre_prose::probe::{FrameCount, Measure, Percentiles, ProbeName, percentiles};

/// How often the summary is printed.
const SUMMARY_EVERY: ambre_core::PageMillis = ambre_core::PageMillis::new(5_000.0);

/// The page's clock, in milliseconds since the page opened.
pub(crate) fn now() -> ambre_core::PageTime {
    ambre_core::PageTime::new(
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.now()),
    )
}

/// The page's clock as a view takes it: the grid reads no clock of its own.
pub(crate) struct PageClock;

impl ambre_draw::Clock for PageClock {
    fn now(&self) -> ambre_core::PageTime {
        now()
    }
}

/// Where the probes' measures go.
pub(crate) struct Timeline {
    /// The samples per measure, kept only under `?probes`.
    samples: Option<BTreeMap<ProbeName, Vec<ambre_core::PageMillis>>>,
    /// The samples per count, kept only under `?probes`.
    counts: Option<BTreeMap<ProbeName, Vec<FrameCount>>>,
    last_summary: ambre_core::PageTime,
    /// A sample landed since the last summary.
    fresh: bool,
}

impl Timeline {
    pub(crate) fn new() -> Self {
        let wanted = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .is_some_and(|search| {
                search
                    .trim_start_matches('?')
                    .split('&')
                    .any(|pair| pair.split('=').next() == Some("probes"))
            });
        Self {
            samples: wanted.then(BTreeMap::new),
            counts: wanted.then(BTreeMap::new),
            last_summary: ambre_core::PageTime::new(0.0),
            fresh: false,
        }
    }

    /// Stamp each measure on the timeline, and keep it under `?probes`.
    pub(crate) fn record(&mut self, measures: Vec<Measure>) {
        for measure in measures {
            stamp(&measure);
            if let Some(samples) = self.samples.as_mut() {
                samples
                    .entry(measure.name)
                    .or_default()
                    .push(measure.duration());
                self.fresh = true;
            }
        }
    }

    /// Stamp each count as a mark carrying it in its detail, and keep it
    /// under `?probes`.
    pub(crate) fn count(&mut self, name: ProbeName, values: Vec<FrameCount>) {
        for value in values {
            mark_count(name.as_str(), value);
            if let Some(counts) = self.counts.as_mut() {
                counts.entry(name).or_default().push(value);
                self.fresh = true;
            }
        }
    }

    /// Print the summary when one is due and something new was measured.
    pub(crate) fn summarize(&mut self, now: ambre_core::PageTime) {
        let Some(samples) = self.samples.as_ref() else {
            return;
        };
        if !self.fresh || now - self.last_summary < SUMMARY_EVERY {
            return;
        }
        self.last_summary = now;
        self.fresh = false;
        for (name, values) in samples {
            if let Some(Percentiles { median, p95 }) =
                percentiles(values, ambre_core::PageMillis::total_cmp)
            {
                let (p50, p95) = (median.get(), p95.get());
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "{}: n={} p50={p50:.2} ms p95={p95:.2} ms",
                    name.as_str(),
                    values.len()
                )));
            }
        }
        for (name, values) in self.counts.iter().flatten() {
            if let Some(Percentiles { median, p95 }) = percentiles(values, FrameCount::cmp) {
                let (p50, p95) = (median.get(), p95.get());
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "{}: n={} p50={p50} frames p95={p95} frames",
                    name.as_str(),
                    values.len()
                )));
            }
        }
    }
}

/// One `performance.mark(name)` at the page's current time. Always on, like
/// the measures.
pub(crate) fn mark(name: ProbeName) {
    if let Some(performance) = web_sys::window().and_then(|w| w.performance()) {
        let _ = performance.mark(name.as_str());
    }
}

/// One `performance.mark(name, { detail: value })`, through `Reflect` like
/// the measures.
fn mark_count(name: &str, value: FrameCount) {
    let Some(performance) = web_sys::window().and_then(|w| w.performance()) else {
        return;
    };
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"detail".into(), &value.get().into());
    let Ok(function) = js_sys::Reflect::get(&performance, &"mark".into()) else {
        return;
    };
    let Ok(function) = function.dyn_into::<js_sys::Function>() else {
        return;
    };
    let _ = function.call2(&performance, &name.into(), &options);
}

/// One `performance.measure(name, { start, end })`. The options form is not
/// in the bindings, so it goes through `Reflect`. A browser without it
/// records nothing, and typing goes on.
fn stamp(measure: &Measure) {
    let Some(performance) = web_sys::window().and_then(|w| w.performance()) else {
        return;
    };
    let options = js_sys::Object::new();
    let set = |key: &str, value: f64| {
        let _ = js_sys::Reflect::set(&options, &key.into(), &value.into());
    };
    set("start", measure.start.get());
    set("end", measure.end.get());
    let Ok(function) = js_sys::Reflect::get(&performance, &"measure".into()) else {
        return;
    };
    let Ok(function) = function.dyn_into::<js_sys::Function>() else {
        return;
    };
    let _ = function.call2(&performance, &measure.name.as_str().into(), &options);
}
