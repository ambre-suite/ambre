// The probe of a call proof (crates/ambre-belt/src/proof.rs, specs/1-Foundations/ambre-spec-call-proofs.md).
//
// The proof host injects this script as the first script of the page, so it runs before the
// application boots. It reads what the shipped page already exposes and changes no behaviour:
//
// * the peer connection of the call, for the counters of `getStats()`;
// * the `ambre:video-paint` measures the UI thread stamps on its timeline;
// * the call worker, which it starts through a wrapper that times each chunk (call_shim.js);
// * the Sync worker, to which it posts the command that seals a file, on the host's order;
// * the local audio tracks, for their echo cancellation setting.
//
// Every 2 s it posts one report to the host, and the answer says whether to seal.
(() => {
  const proof = {
    pcs: [],
    tracks: [],
    worker: null,
    sync: null,
    seal: null,
    sealing: false,
    pending: null,
    seals: { done: 0, failed: 0, ms: [] },
  };
  window.ambreProof = proof;

  const P = window.RTCPeerConnection;
  if (P) {
    window.RTCPeerConnection = class extends P {
      constructor(...args) {
        super(...args);
        proof.pcs.push(this);
      }
    };
  }

  const begins = (bytes, prefix) => {
    if (!prefix || bytes.length < prefix.length) return false;
    for (let i = 0; i < prefix.length; i += 1) if (bytes[i] !== prefix[i]) return false;
    return true;
  };

  // One seal at a time, the next as soon as the Sync worker answers the last one.
  const sealNext = () => {
    if (!proof.sealing || !proof.sync || !proof.seal || proof.pending !== null) return;
    const bytes = new Uint8Array(proof.seal.command);
    proof.pending = performance.now();
    proof.sync.postMessage(bytes.buffer, [bytes.buffer]);
  };
  const onSync = (e) => {
    if (proof.pending === null || !(e.data instanceof ArrayBuffer)) return;
    const bytes = new Uint8Array(e.data);
    const sealed = begins(bytes, proof.seal.sealed);
    const failed = begins(bytes, proof.seal.failed);
    if (!sealed && !failed) return;
    proof.seals.ms.push(performance.now() - proof.pending);
    if (sealed) proof.seals.done += 1;
    else proof.seals.failed += 1;
    proof.pending = null;
    sealNext();
  };

  const W = window.Worker;
  window.Worker = class extends W {
    constructor(url, options) {
      const href = String(url);
      const call = href.includes('ambre-call');
      super(call ? '/ambre-proof-call-worker.js?loader=' + encodeURIComponent(href) : url, options);
      if (call) {
        this.addEventListener('message', (e) => {
          if (e.data && e.data.ambreProof) proof.worker = e.data.ambreProof;
        });
      }
      if (href.includes('ambre-sync')) {
        proof.sync = this;
        this.addEventListener('message', onSync);
      }
    }
  };

  const devices = window.MediaDevices && MediaDevices.prototype;
  if (devices && devices.getUserMedia) {
    const G = devices.getUserMedia;
    devices.getUserMedia = function (constraints) {
      return G.call(this, constraints).then((stream) => {
        stream.getTracks().forEach((t) => proof.tracks.push(t));
        return stream;
      });
    };
  }

  // The GPU the page draws with, so a figure of a desktop names it, and a software
  // renderer shows.
  const renderer = (() => {
    try {
      const gl = document.createElement('canvas').getContext('webgl2');
      const info = gl && gl.getExtension('WEBGL_debug_renderer_info');
      return info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : (gl ? 'unnamed' : 'no WebGL2');
    } catch (e) {
      return 'no WebGL2: ' + e;
    }
  })();

  fetch('/proof/seal').then((r) => r.json()).then((seal) => { proof.seal = seal; });

  const stats = async () => {
    const pc = proof.pcs.filter((p) => p.connectionState !== 'closed').pop();
    const out = { videosIn: [], videosOut: [], audioIn: [], state: pc ? pc.connectionState : null };
    if (!pc) return out;
    const all = await pc.getStats();
    all.forEach((s) => {
      if (s.type === 'inbound-rtp' && s.kind === 'video') {
        out.videosIn.push({ id: s.id, width: s.frameWidth || 0, height: s.frameHeight || 0,
          decoded: s.framesDecoded || 0, dropped: s.framesDropped || 0,
          freezes: s.freezeCount === undefined ? null : s.freezeCount,
          frozen: s.totalFreezesDuration === undefined ? null : s.totalFreezesDuration });
      }
      if (s.type === 'outbound-rtp' && s.kind === 'video') {
        out.videosOut.push({ rid: s.rid || null, width: s.frameWidth || 0,
          encoded: s.framesEncoded || 0, limit: s.qualityLimitationReason || null });
      }
      if (s.type === 'inbound-rtp' && s.kind === 'audio') {
        out.audioIn.push({ id: s.id, samples: s.totalSamplesReceived || 0,
          concealed: s.concealedSamples || 0, packets: s.packetsReceived || 0 });
      }
    });
    return out;
  };

  const report = async () => {
    const paints = performance.getEntriesByName('ambre:video-paint').map((e) => e.duration);
    performance.clearMeasures('ambre:video-paint');
    const seals = proof.seals;
    proof.seals = { done: 0, failed: 0, ms: [] };
    const body = Object.assign(await stats(), {
      at: performance.now(),
      wall: Date.now(),
      agent: navigator.userAgent,
      renderer,
      home: !!document.querySelector('.ambre-doc-list'),
      visible: document.visibilityState,
      paints,
      worker: proof.worker,
      seals,
      sealing: proof.sealing,
      echo: proof.tracks.filter((t) => t.kind === 'audio' && t.readyState === 'live')
        .map((t) => (t.getSettings ? t.getSettings().echoCancellation : null)),
    });
    try {
      const answer = await (await fetch('/proof/report', { method: 'POST',
        headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })).json();
      proof.sealing = !!answer.seal;
      sealNext();
    } catch (e) {
      // The host stopped: the page goes on without it.
    }
  };
  setInterval(report, 2000);
})();
