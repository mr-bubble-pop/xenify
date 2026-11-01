// Recording a call, on this computer: the sound and pictures the call
// already opens here, of the people in the recording only, made into one
// video (WebM: VP8 pictures, Opus sound). Each person's own sound goes, a
// piece at a time, to `hooks.piece` for the transcript. With `hooks.video`
// false only the transcript is made (no video is kept).

const W = 960, H = 540, FPS = 10;
const RATE = 48000;
/** 20 ms of sound, as the call's pieces come. */
const FRAME = 960;
/** Transcript pieces of one person at 16 kHz: cut at a pause (400 ms of
 *  quiet) once 8 s long, or at 28 s however it goes: a cut in the middle of
 *  a word loses it on both sides. */
const PIECE_MIN = 8 * 16000, PIECE = 28 * 16000, PAUSE_MS = 400;
/** 48 kHz → 16 kHz: a small low-pass first (a 3-sample average lets sounds
 *  above 8 kHz fold back in and muddy the words). Taps sum to 1. */
const LOWPASS = (() => {
  const t = [-0.0106, 0, 0.1162, 0.25, 0.3, 0.25, 0.1162, 0, -0.0106];
  const sum = t.reduce((a, b) => a + b, 0);
  return t.map((x) => x / sum);
})();
/** Soft limiting: loud moments (two people at once, a hot microphone) are
 *  rounded off above 0.8 instead of clipped flat, which sounds broken. */
const soft = (x) => {
  const a = Math.abs(x);
  return a <= 0.8 ? x : Math.sign(x) * (0.8 + 0.2 * Math.tanh((a - 0.8) / 0.2));
};
/** A save step that never answers (seen with WebKitGTK's encoders) mustn't
 *  hold the recording up for ever. */
const within = (p, ms) => Promise.race([p, new Promise((ok) => setTimeout(ok, ms))]);

export class Recording {
  /// `call`: the Call. `hooks.included()`: ids of the people in it now;
  /// `hooks.name(id)`; `hooks.piece(id, atMs, samples16k)` (Int16Array);
  /// `hooks.error(text)`.
  constructor(call, hooks) {
    this.call = call;
    this.hooks = hooks;
    this.queues = new Map(); // id → [Float32Array of FRAME]
    this.speakers = new Map(); // id → { at (frame number), buf: Int16Array, n, loud }
    this.chunks = []; // { track, ts (µs), key, data }
    this.frames = 0; // sound frames mixed so far
    this.opusHead = null;
    this.keep = hooks.video !== false; // the video, or only the transcript
  }

  get secs() {
    return this.t0 ? (performance.now() - this.t0) / 1000 : 0;
  }

  start() {
    this.t0 = performance.now();
    this.call.listen = (id, samples) => {
      if (!this.hooks.included().has(id)) return;
      let q = this.queues.get(id);
      if (!q) this.queues.set(id, (q = []));
      // Pieces come in 20 ms; keep at most a second waiting.
      for (let i = 0; i + FRAME <= samples.length; i += FRAME) q.push(samples.subarray(i, i + FRAME));
      while (q.length > 50) q.shift();
    };
    if (this.keep) this.startEncoders();
    this.mixer = setInterval(() => this.mix(), 100);
  }

  startEncoders() {
    this.audio = new AudioEncoder({
      output: (chunk, meta) => {
        if (meta?.decoderConfig?.description && !this.opusHead) this.opusHead = new Uint8Array(meta.decoderConfig.description);
        const b = new Uint8Array(chunk.byteLength);
        chunk.copyTo(b);
        this.chunks.push({ track: 2, ts: chunk.timestamp, key: true, data: b });
      },
      error: (e) => this.hooks.error(`Recording the sound: ${e.message || e}`),
    });
    this.audio.configure({ codec: "opus", sampleRate: RATE, numberOfChannels: 1, bitrate: 48000 });
    this.video = new VideoEncoder({
      output: (chunk) => {
        const b = new Uint8Array(chunk.byteLength);
        chunk.copyTo(b);
        this.chunks.push({ track: 1, ts: chunk.timestamp, key: chunk.type === "key", data: b });
      },
      error: (e) => this.hooks.error(`Recording the pictures: ${e.message || e}`),
    });
    this.video.configure({ codec: "vp8", width: W, height: H, bitrate: 800_000, framerate: FPS });
    this.canvas = document.createElement("canvas");
    this.canvas.width = W;
    this.canvas.height = H;
    this.g = this.canvas.getContext("2d");
    this.pictures = 0;
    this.drawer = setInterval(() => this.draw(), 1000 / FPS);
  }

  /** Mixes the sound that's due: one frame per 20 ms since the start. */
  mix() {
    const due = Math.floor((performance.now() - this.t0) / 20);
    const people = this.hooks.included();
    // Timers held back (the window hidden or minimised) leave many frames
    // due at once: catching up on all of them at once froze the window. Any
    // sound waiting is at most a second old, so the rest is a gap: skipped,
    // with each person's transcript piece ended where it was.
    if (due - this.frames > 100) {
      for (const [id, sp] of this.speakers) this.endPiece(id, sp);
      this.speakers.clear();
      this.frames = due - 50;
    }
    while (this.frames < due) {
      const out = new Float32Array(FRAME);
      for (const id of people) {
        const piece = this.queues.get(id)?.shift();
        if (piece) for (let i = 0; i < FRAME; i++) out[i] += piece[i];
        this.hear(id, piece);
      }
      for (let i = 0; i < FRAME; i++) out[i] = soft(out[i]);
      if (this.audio?.state === "configured") {
        const data = new AudioData({ format: "f32-planar", sampleRate: RATE, numberOfFrames: FRAME, numberOfChannels: 1, timestamp: this.frames * 20000, data: out });
        this.audio.encode(data);
        data.close();
      }
      this.frames++;
    }
  }

  /** One person's 20 ms (or silence) for their part of the transcript. */
  hear(id, piece) {
    let s = this.speakers.get(id);
    if (!s) this.speakers.set(id, (s = { at: this.frames, buf: new Int16Array(PIECE), n: 0, loud: false, quiet: 0, hist: new Float32Array(LOWPASS.length) }));
    // 48 kHz → 16 kHz: low-passed, then every third sample. `hist` carries
    // the last few samples over from the frame before.
    const h = s.hist, L = LOWPASS.length;
    const x = new Float32Array(L - 1 + FRAME);
    x.set(h.subarray(1));
    if (piece) x.set(piece, L - 1);
    let frameLoud = false;
    for (let i = 0; i < FRAME / 3; i++) {
      let v = 0;
      for (let k = 0; k < L; k++) v += LOWPASS[k] * x[3 * i + k];
      if (Math.abs(v) > 0.02) frameLoud = true;
      s.buf[s.n++] = Math.max(-1, Math.min(1, v)) * 32767;
    }
    h.set(x.subarray(x.length - L));
    if (frameLoud) { s.loud = true; s.quiet = 0; } else s.quiet += 20;
    if ((s.n >= PIECE_MIN && s.quiet >= PAUSE_MS) || s.n + FRAME / 3 > PIECE) this.endPiece(id, s);
  }

  endPiece(id, s) {
    // Only pieces with something said in them are turned into text.
    if (s.loud && s.n > 8000) this.hooks.piece(id, s.at * 20, s.buf.slice(0, s.n));
    this.speakers.set(id, { at: this.frames + 1, buf: s.buf, n: 0, loud: false, quiet: 0, hist: s.hist });
  }

  /** The picture: a shared screen big, or everyone's tile. */
  draw() {
    if (this.video.state !== "configured" || this.video.encodeQueueSize > 2) return;
    const g = this.g;
    const call = this.call;
    const here = new Set(call.info.people.map((u) => u.id));
    const ids = [...this.hooks.included()].filter((id) => here.has(id));
    g.fillStyle = "#1E1C1A";
    g.fillRect(0, 0, W, H);
    const sharing = call.sharing != null && ids.includes(call.sharing) ? call.sharing : null;
    const screen = sharing != null ? call.people.get(sharing)?.screen : null;
    if (screen) {
      fit(g, screen, 0, 0, W - 200, H);
      ids.forEach((id, i) => this.tile(id, W - 192, 8 + i * 112, 184, 104));
    } else if (ids.length) {
      const cols = Math.ceil(Math.sqrt(ids.length));
      const rows = Math.ceil(ids.length / cols);
      const tw = (W - 8 * (cols + 1)) / cols, th = (H - 8 * (rows + 1)) / rows;
      ids.forEach((id, i) => this.tile(id, 8 + (i % cols) * (tw + 8), 8 + Math.floor(i / cols) * (th + 8), tw, th));
    } else {
      g.fillStyle = "#A39E98";
      g.font = "600 20px sans-serif";
      g.textAlign = "center";
      g.fillText("Waiting for someone to turn their microphone or camera on", W / 2, H / 2);
      g.textAlign = "start";
    }
    const frame = new VideoFrame(this.canvas, { timestamp: Math.round((performance.now() - this.t0) * 1000) });
    this.video.encode(frame, { keyFrame: this.pictures % (FPS * 2) === 0 });
    frame.close();
    this.pictures++;
  }

  tile(id, x, y, w, h) {
    const g = this.g;
    const call = this.call;
    const p = call.people.get(id) || {};
    const camOn = id === call.me ? call.cam : p.lastCam && performance.now() - p.lastCam < 3000;
    g.fillStyle = "#4A433E";
    g.fillRect(x, y, w, h);
    if (camOn && p.cam) fit(g, p.cam, x, y, w, h);
    else {
      const name = this.hooks.name(id);
      const r = Math.min(w, h) / 5;
      g.fillStyle = "#C2410C";
      g.beginPath();
      g.arc(x + w / 2, y + h / 2, r, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = "#fff";
      g.font = `700 ${Math.round(r * 0.8)}px sans-serif`;
      g.textAlign = "center";
      g.textBaseline = "middle";
      g.fillText(name.split(/\s+/).slice(0, 2).map((s) => s[0] || "").join("").toUpperCase(), x + w / 2, y + h / 2);
      g.textAlign = "start";
      g.textBaseline = "alphabetic";
    }
    const name = this.hooks.name(id);
    g.font = "600 13px sans-serif";
    const tw = g.measureText(name).width + 12;
    g.fillStyle = "rgba(0,0,0,0.55)";
    g.fillRect(x + 6, y + h - 26, tw, 20);
    g.fillStyle = "#fff";
    g.fillText(name, x + 12, y + h - 11);
  }

  /** Stops: the video, and how long it is. The last transcript pieces go to `hooks.piece`. */
  async finish() {
    clearInterval(this.mixer);
    clearInterval(this.drawer);
    this.mix();
    this.call.listen = null;
    for (const [id, s] of this.speakers) this.endPiece(id, s);
    const secs = this.frames / 50;
    if (!this.keep) return { webm: null, secs: Math.round(secs) };
    try { await within(this.audio.flush(), 5000); } catch {}
    try { await within(this.video.flush(), 5000); } catch {}
    try { this.audio.close(); } catch {}
    try { this.video.close(); } catch {}
    return { webm: webm(this.chunks, this.opusHead, secs * 1000), secs: Math.round(secs) };
  }

  cancel() {
    clearInterval(this.mixer);
    clearInterval(this.drawer);
    this.call.listen = null;
    try { this.audio?.close(); } catch {}
    try { this.video?.close(); } catch {}
  }
}

/** Draws `src` into the box, keeping its shape. */
function fit(g, src, x, y, w, h) {
  const sw = src.videoWidth || src.width, sh = src.videoHeight || src.height;
  if (!sw || !sh) return;
  const k = Math.min(w / sw, h / sh);
  g.drawImage(src, x + (w - sw * k) / 2, y + (h - sh * k) / 2, sw * k, sh * k);
}

// ---- WebM (Matroska) ----
// Elements are kept as lists of parts (nothing big is copied): [id, size, …body].

const enc = new TextEncoder();
const len = (parts) => parts.reduce((n, p) => n + p.byteLength, 0);

function idBytes(id) {
  const out = [];
  for (let v = id; v > 0; v = Math.floor(v / 256)) out.unshift(v & 0xff);
  return new Uint8Array(out);
}

/** A size, always in 8 bytes. */
function sizeBytes(n) {
  const out = new Uint8Array(8);
  out[0] = 0x01;
  for (let i = 7; i > 0; i--, n = Math.floor(n / 256)) out[i] = n & 0xff;
  return out;
}

function el(id, parts) {
  return [idBytes(id), sizeBytes(len(parts)), ...parts];
}

function uint(id, v) {
  const out = [];
  do {
    out.unshift(v & 0xff);
    v = Math.floor(v / 256);
  } while (v > 0);
  return el(id, [new Uint8Array(out)]);
}

function float(id, v) {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setFloat64(0, v);
  return el(id, [b]);
}

const str = (id, s) => el(id, [enc.encode(s)]);

/** A number in 8 bytes (positions, written before they're known). */
function uint8(id, v) {
  const b = new Uint8Array(8);
  for (let i = 7; i >= 0; i--, v = Math.floor(v / 256)) b[i] = v & 0xff;
  return el(id, [b]);
}

function opusHeadFor() {
  const b = new Uint8Array(19);
  b.set(enc.encode("OpusHead"));
  const v = new DataView(b.buffer);
  b[8] = 1; // version
  b[9] = 1; // channels
  v.setUint16(10, 312, true); // pre-skip
  v.setUint32(12, RATE, true);
  return b;
}

/** The recorded chunks as a WebM file (a Blob). */
export function webm(chunks, opusHead, durationMs) {
  const header = el(0x1a45dfa3, [
    ...uint(0x4286, 1), ...uint(0x42f7, 1), ...uint(0x42f2, 4), ...uint(0x42f3, 8),
    ...str(0x4282, "webm"), ...uint(0x4287, 2), ...uint(0x4285, 2),
  ]);
  const info = el(0x1549a966, [...uint(0x2ad7b1, 1_000_000), ...str(0x4d80, "Xenify Chat"), ...str(0x5741, "Xenify Chat"), ...float(0x4489, durationMs)]);
  const tracks = el(0x1654ae6b, [
    ...el(0xae, [...uint(0xd7, 1), ...uint(0x73c5, 1), ...uint(0x83, 1), ...str(0x86, "V_VP8"), ...el(0xe0, [...uint(0xb0, W), ...uint(0xba, H)])]),
    ...el(0xae, [...uint(0xd7, 2), ...uint(0x73c5, 2), ...uint(0x83, 2), ...str(0x86, "A_OPUS"), ...el(0x63a2, [opusHead || opusHeadFor()]), ...el(0xe1, [...float(0xb5, RATE), ...uint(0x9f, 1)])]),
  ]);
  // In time order (pictures before sound at the same moment); a new cluster
  // at each key picture (every 2 s) or every 30 s at most.
  const sorted = chunks.slice().sort((a, b) => a.ts - b.ts || a.track - b.track);
  const clusters = [];
  const starts = []; // { at (ms), pos (bytes into the clusters) }
  let cluster = null;
  const close = () => {
    if (!cluster) return;
    starts.push({ at: cluster.at, pos: len(clusters) });
    clusters.push(...el(0x1f43b675, cluster.parts));
  };
  for (const c of sorted) {
    const ms = Math.round(c.ts / 1000);
    if (!cluster || (c.track === 1 && c.key && ms > cluster.at) || ms - cluster.at > 30_000) {
      close();
      cluster = { at: ms, parts: uint(0xe7, ms) };
    }
    const head = new Uint8Array(4);
    head[0] = 0x80 | c.track;
    new DataView(head.buffer).setInt16(1, ms - cluster.at);
    head[3] = c.key ? 0x80 : 0;
    cluster.parts.push(...el(0xa3, [head, c.data]));
  }
  close();
  // Where things are, for players that jump about: a seek head first, the
  // cues (a cluster's time and place) last. Places count from the
  // segment's start.
  const seek = (id, pos) => el(0x4dbb, [...el(0x53ab, [idBytes(id)]), ...uint8(0x53ac, pos)]);
  const seekLen = len(el(0x114d9b74, [...seek(0x1549a966, 0), ...seek(0x1654ae6b, 0), ...seek(0x1c53bb6b, 0)]));
  const at0 = seekLen + len(info) + len(tracks);
  const cues = el(0x1c53bb6b, starts.flatMap((c) => el(0xbb, [...uint(0xb3, c.at), ...el(0xb7, [...uint(0xf7, 1), ...uint(0xf1, at0 + c.pos)])])));
  const seekHead = el(0x114d9b74, [...seek(0x1549a966, seekLen), ...seek(0x1654ae6b, seekLen + len(info)), ...seek(0x1c53bb6b, at0 + len(clusters))]);
  const segment = el(0x18538067, [...seekHead, ...info, ...tracks, ...clusters, ...cues]);
  return new Blob([...header, ...segment], { type: "video/webm" });
}
