// The timer of the call worker in a call proof (crates/ambre-belt/src/proof.rs).
//
// The proof host starts the call worker through a wrapper that imports this script, then the
// shipped loader. So the shipped worker runs unchanged, and this script times what it does:
//
// * a chunk is held from the moment the worker's read of it resolves to the moment the worker
//   writes it on. That span covers the seal or the open, and the copies around it;
// * the held time of the worker is the union of those spans over the wall clock. The worker
//   has one thread, so the union bounds from above the time its transforms run.
//
// Every 2 s it posts its running totals to the page as `{ ambreProof: ... }`. The page of the
// application reads neither a number nor a byte array in it, and drops it.
(() => {
  const BIN = 0.05;
  const BINS = 400;
  const totals = {
    bin: BIN,
    hist: new Array(BINS).fill(0),
    over: 0,
    max: 0,
    held: 0,
    counts: {},
    at: performance.now(),
  };
  let until = 0;
  const started = new WeakMap();
  const readers = new WeakMap();
  const streams = new WeakMap();

  const record = (kind, t0, t1) => {
    const span = t1 - t0;
    const bin = Math.floor(span / BIN);
    if (bin < BINS) totals.hist[bin] += 1;
    else totals.over += 1;
    totals.max = Math.max(totals.max, span);
    totals.held += Math.max(0, t1 - Math.max(t0, until));
    until = Math.max(until, t1);
    totals.counts[kind] = (totals.counts[kind] || 0) + 1;
  };

  const getReader = ReadableStream.prototype.getReader;
  ReadableStream.prototype.getReader = function (...args) {
    const reader = getReader.apply(this, args);
    if (streams.has(this)) readers.set(reader, streams.get(this));
    return reader;
  };
  const read = ReadableStreamDefaultReader.prototype.read;
  ReadableStreamDefaultReader.prototype.read = function (...args) {
    const kind = readers.get(this);
    const pending = read.apply(this, args);
    if (kind === undefined) return pending;
    return pending.then((r) => {
      if (!r.done && r.value) started.set(r.value, { kind, t0: performance.now() });
      return r;
    });
  };
  const write = WritableStreamDefaultWriter.prototype.write;
  WritableStreamDefaultWriter.prototype.write = function (chunk, ...args) {
    const held = chunk && started.get(chunk);
    if (held) {
      record(held.kind, held.t0, performance.now());
      started.delete(chunk);
    }
    return write.call(this, chunk, ...args);
  };

  // The worker learns of each transform from an event: the readable of each one is marked
  // with its side and its track before the worker reads it.
  const add = self.addEventListener.bind(self);
  self.addEventListener = function (type, listener, options) {
    if (type !== 'rtctransform') return add(type, listener, options);
    return add(type, (event) => {
      const t = event.transformer;
      const o = (t && t.options) || {};
      const track = ['audio', 'video', 'screen'][o.track === undefined ? 0 : o.track] || 'unknown';
      if (t && t.readable) streams.set(t.readable, (o.side || 'unknown') + ' ' + track);
      return listener.call(self, event);
    }, options);
  };

  setInterval(() => {
    self.postMessage({ ambreProof: Object.assign({}, totals, { at: performance.now() }) });
  }, 2000);
})();
