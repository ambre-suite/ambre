// The figures of the Murmure phone measures that only the running application
// holds (`specs/5-Murmure/features/ambre-spec-murmure-phone-measures.md`).
//
// Paste this file into the console of the page of the phone stack, through
// the remote inspector of the phone's browser. It reads three figures and
// prints one `PHONE-MEASURE` line of JSON:
//
// * the color emoji font, as the network served it;
// * the local store of the origin, as the browser counts it;
// * the memory of the page and its workers, where the browser can measure it.
//
// It changes nothing in the page: it reads timings and estimates alone.
(async () => {
  const font = performance
    .getEntriesByType("resource")
    .filter((entry) => entry.name.includes("NotoColorEmoji"))
    .map((entry) => ({
      transfer_bytes: entry.transferSize,
      encoded_bytes: entry.encodedBodySize,
      decoded_bytes: entry.decodedBodySize,
      fetch_ms: Math.round(entry.duration),
    }));
  const store = navigator.storage && navigator.storage.estimate
    ? await navigator.storage.estimate()
    : null;
  // Chrome measures the page and its workers when the page is isolated, as
  // the application is. Safari has no such call: the Timelines tab of its
  // Web Inspector reads the memory instead.
  let memory = null;
  if (crossOriginIsolated && performance.measureUserAgentSpecificMemory) {
    const measured = await performance.measureUserAgentSpecificMemory();
    memory = {
      bytes: measured.bytes,
      parts: measured.breakdown
        .filter((part) => part.bytes > 0)
        .map((part) => ({ bytes: part.bytes, types: part.types })),
    };
  }
  const report = {
    page: "application",
    user_agent: navigator.userAgent,
    emoji_font: font.length > 0 ? font[0] : "not loaded: show an emoji first",
    local_store_bytes: store ? store.usage : "no estimate in this browser",
    memory: memory ?? "no measure in this browser: read the Web Inspector",
  };
  console.log("PHONE-MEASURE " + JSON.stringify(report));
})();
