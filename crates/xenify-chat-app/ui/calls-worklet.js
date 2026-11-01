// Sound for calls, off the screens' thread.
//  - "capture": the microphone in 20 ms pieces (960 samples at 48 kHz).
//  - "player": everyone else's sound, mixed; pieces arrive per person and
//    play in order, with a little room so short delays don't click.

class Capture extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buf = new Float32Array(960);
    this.at = 0;
  }
  process(inputs) {
    const ch = inputs[0]?.[0];
    if (ch) {
      for (let i = 0; i < ch.length; i++) {
        this.buf[this.at++] = ch[i];
        if (this.at === this.buf.length) {
          this.port.postMessage(this.buf.slice());
          this.at = 0;
        }
      }
    }
    return true;
  }
}

// What's played is kept to a comfortable level, whatever someone's
// microphone sends (one turned right up, or boosted by the computer, was
// deafening at half volume):
//  - each person's voice is brought to about the same speaking level:
//    turned down quickly when it's too loud, up only a little and slowly;
//  - nothing played goes over CEILING (a limiter on the mix).
const TARGET = 0.08; // a voice's level as played (RMS, about -22 dBFS)
const CEILING = 0.5; // the loudest anything is played (-6 dBFS)
const MAX_UP = 2; // a quiet voice is made at most this much louder
const MIN_DOWN = 0.05; // and a loud one up to 20 times quieter

class Player extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queues = new Map(); // sender -> [Float32Array…], with a read position
    this.limit = 1; // the limiter's gain now
    this.port.onmessage = (e) => {
      const { sender, samples } = e.data;
      let q = this.queues.get(sender);
      if (!q) this.queues.set(sender, (q = { list: [], pos: 0, started: false, ms: TARGET * TARGET, gain: 1, buf: null }));
      q.list.push(samples);
      // Too far behind (over half a second): drop the oldest.
      while (q.list.length > 25) q.list.shift();
    };
  }
  process(_, outputs) {
    const out = outputs[0][0];
    out.fill(0);
    for (const q of this.queues.values()) {
      // Wait for 60 ms before starting, so the first pieces aren't cut.
      if (!q.started && q.list.length < 3) continue;
      q.started = true;
      if (!q.buf || q.buf.length !== out.length) q.buf = new Float32Array(out.length);
      const mine = q.buf;
      mine.fill(0);
      let i = 0;
      while (i < out.length && q.list.length) {
        const cur = q.list[0];
        const n = Math.min(out.length - i, cur.length - q.pos);
        mine.set(cur.subarray(q.pos, q.pos + n), i);
        i += n;
        q.pos += n;
        if (q.pos >= cur.length) {
          q.list.shift();
          q.pos = 0;
        }
      }
      if (!q.list.length) q.started = false;
      // How loud this person is: follows a louder voice at once, a quieter
      // one slowly (and silence not at all, so pauses don't turn them up).
      let sum = 0;
      for (let k = 0; k < mine.length; k++) sum += mine[k] * mine[k];
      const ms = sum / mine.length;
      if (ms > q.ms) q.ms += (ms - q.ms) * 0.3;
      else if (ms > 1e-5) q.ms += (ms - q.ms) * 0.01;
      const want = Math.min(MAX_UP, Math.max(MIN_DOWN, TARGET / Math.sqrt(q.ms)));
      // Down quickly, up slowly; changed smoothly across the block.
      const next = want < q.gain ? q.gain + (want - q.gain) * 0.5 : q.gain + (want - q.gain) * 0.005;
      const step = (next - q.gain) / mine.length;
      for (let k = 0; k < mine.length; k++) out[k] += mine[k] * (q.gain + step * k);
      q.gain = next;
    }
    // The ceiling: turned down at once for a peak, back up over ~50 ms.
    for (let k = 0; k < out.length; k++) {
      const a = Math.abs(out[k]) * this.limit;
      if (a > CEILING) this.limit = CEILING / Math.abs(out[k]);
      out[k] *= this.limit;
      this.limit += (1 - this.limit) * 0.0005;
    }
    for (let c = 1; c < outputs[0].length; c++) outputs[0][c].set(out);
    return true;
  }
}

registerProcessor("capture", Capture);
registerProcessor("player", Player);
