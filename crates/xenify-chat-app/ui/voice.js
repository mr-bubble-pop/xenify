// Voice messages: recording from the microphone, the text as you talk
// (turned into text on this computer, a piece at a time), and the sound
// file that's sent (WAV: 16 kHz, mono, 16-bit, which plays everywhere).

export const RATE = 16000;
/** Bars in a voice message's picture of its sound. */
export const BARS = 32;
/** Text is made for up to this much at a time (the models take 30 s). */
const PIECE = 25 * RATE;

export class Recorder {
  /// `hooks.changed()` redraws; `hooks.text(base64 samples)` → the text, or
  /// null when there's no model; `hooks.error(text)`; `hooks.testAudio`:
  /// base64 samples to "hear" instead of the microphone (tests).
  constructor(hooks) {
    this.hooks = hooks;
    this.chunks = []; // Float32Array pieces at 16 kHz
    this.length = 0;
    this.levels = []; // loudness per 0.1 s, 0..1
    this.paused = false;
    this.done = []; // text of finished pieces
    this.live = ""; // text of the piece going on
    this.from = 0; // where that piece starts (samples)
    this.busy = null; // the text being made
    this.noModel = false;
  }

  get secs() {
    return this.length / RATE;
  }

  get text() {
    return [...this.done, this.live].filter(Boolean).join(" ");
  }

  async start() {
    if (this.hooks.testAudio) {
      const all = fromBase64(this.hooks.testAudio);
      let at = 0;
      this.feeder = setInterval(() => {
        if (this.paused) return;
        const n = RATE / 10;
        const piece = new Float32Array(n);
        for (let i = 0; i < n && at < all.length; i++, at++) piece[i] = all[at] / 32768;
        this.add(piece);
      }, 100);
    } else {
      this.stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
      this.ctx = new AudioContext({ sampleRate: 48000 });
      await this.ctx.audioWorklet.addModule("calls-worklet.js");
      this.node = new AudioWorkletNode(this.ctx, "capture");
      // 48 kHz → 16 kHz: every three samples averaged.
      this.node.port.onmessage = (e) => {
        if (this.paused) return;
        const s = e.data;
        const out = new Float32Array(Math.floor(s.length / 3));
        for (let i = 0; i < out.length; i++) out[i] = (s[3 * i] + s[3 * i + 1] + s[3 * i + 2]) / 3;
        this.add(out);
      };
      this.ctx.createMediaStreamSource(this.stream).connect(this.node);
    }
    this.ticker = setInterval(() => this.makeText(), 2500);
  }

  add(piece) {
    this.chunks.push(piece);
    this.length += piece.length;
    this.pending = (this.pending || []).concat([piece]);
    const pendingLength = this.pending.reduce((n, p) => n + p.length, 0);
    if (pendingLength >= RATE / 10) {
      let sum = 0;
      for (const p of this.pending) for (const v of p) sum += v * v;
      this.levels.push(Math.min(1, Math.sqrt(sum / pendingLength) * 6));
      this.pending = [];
      this.hooks.changed();
    }
  }

  pause() {
    this.paused = !this.paused;
    this.hooks.changed();
  }

  /** Samples `from`..`to` as 16-bit. */
  slice(from, to) {
    const out = new Int16Array(to - from);
    let at = 0;
    for (const c of this.chunks) {
      const a = Math.max(from - at, 0);
      const b = Math.min(to - at, c.length);
      for (let i = a; i < b; i++) out[at + i - from] = Math.max(-1, Math.min(1, c[i])) * 32767;
      at += c.length;
      if (at >= to) break;
    }
    return out;
  }

  /** Text for what's been said since the last finished piece. */
  makeText(last = false) {
    if (this.noModel) return Promise.resolve();
    if (this.busy) return last ? this.busy.then(() => this.makeText(true)) : this.busy;
    // Stopping: the text made a moment ago covers it (under a second since).
    if (last && this.liveTo && this.length - this.liveTo < RATE) {
      if (this.live) this.done.push(this.live);
      this.live = "";
      this.from = this.length;
      return Promise.resolve();
    }
    const to = Math.min(this.length, this.from + PIECE);
    if (to - this.from < RATE / 2) return Promise.resolve();
    const finished = to - this.from >= PIECE || last;
    const from = this.from;
    this.busy = this.hooks
      .text(toBase64(this.slice(from, to)))
      .then((t) => {
        if (t == null) {
          this.noModel = true;
          return;
        }
        if (finished) {
          this.done.push(t.trim());
          this.live = "";
          this.from = to;
          this.liveTo = 0;
        } else {
          this.live = t.trim();
          this.liveTo = to;
        }
        this.hooks.changed();
      })
      .catch((e) => this.hooks.error(String(e?.message || e)))
      .finally(() => {
        this.busy = null;
      });
    return this.busy.then(() => (last && this.from < this.length && !this.noModel ? this.makeText(true) : undefined));
  }

  release() {
    clearInterval(this.ticker);
    clearInterval(this.feeder);
    for (const t of this.stream?.getTracks() || []) t.stop();
    this.ctx?.close().catch(() => {});
  }

  /** Stops: the sound file, how long, its picture and the text. */
  async finish() {
    this.release();
    await this.makeText(true);
    return { wav: wav(this.slice(0, this.length)), secs: Math.round(this.secs), wave: wave(this.levels), text: this.text };
  }

  cancel() {
    this.release();
  }
}

/** The loudness picture: `BARS` numbers, 1..9. */
export function wave(levels) {
  const out = [];
  for (let i = 0; i < BARS; i++) {
    const a = Math.floor((i * levels.length) / BARS);
    const b = Math.max(a + 1, Math.floor(((i + 1) * levels.length) / BARS));
    const part = levels.slice(a, b);
    const v = part.length ? Math.max(...part) : 0;
    out.push(Math.max(1, Math.min(9, Math.round(v * 9))));
  }
  return out;
}

/** 16-bit samples → a WAV file. */
export function wav(samples) {
  const out = new Uint8Array(44 + samples.length * 2);
  const v = new DataView(out.buffer);
  const text = (at, s) => [...s].forEach((c, i) => (out[at + i] = c.charCodeAt(0)));
  text(0, "RIFF");
  v.setUint32(4, 36 + samples.length * 2, true);
  text(8, "WAVE");
  text(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true); // PCM
  v.setUint16(22, 1, true); // mono
  v.setUint32(24, RATE, true);
  v.setUint32(28, RATE * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  text(36, "data");
  v.setUint32(40, samples.length * 2, true);
  new Int16Array(out.buffer, 44).set(samples);
  return out;
}

/** A voice message's WAV → its samples, as base64 (for making its text). */
export function wavSamples(bytes) {
  const b = new Uint8Array(bytes);
  // Our own files: the samples start after a 44-byte header.
  return toBase64(new Int16Array(b.buffer.slice(b.byteOffset + 44, b.byteOffset + b.byteLength - ((b.byteLength - 44) % 2))));
}

export function toBase64(samples) {
  const bytes = new Uint8Array(samples.buffer, samples.byteOffset, samples.byteLength);
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

function fromBase64(text) {
  const s = atob(text);
  const bytes = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i);
  return new Int16Array(bytes.buffer, 0, bytes.length >> 1);
}

/** 38 → "0:38". */
export function clock(secs) {
  const s = Math.max(0, Math.round(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
