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

class Player extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queues = new Map(); // sender -> [Float32Array…], with a read position
    this.port.onmessage = (e) => {
      const { sender, samples } = e.data;
      let q = this.queues.get(sender);
      if (!q) this.queues.set(sender, (q = { list: [], pos: 0, started: false }));
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
      let i = 0;
      while (i < out.length && q.list.length) {
        const cur = q.list[0];
        const n = Math.min(out.length - i, cur.length - q.pos);
        for (let k = 0; k < n; k++) out[i + k] += cur[q.pos + k];
        i += n;
        q.pos += n;
        if (q.pos >= cur.length) {
          q.list.shift();
          q.pos = 0;
        }
      }
      if (!q.list.length) q.started = false;
    }
    for (let c = 1; c < outputs[0].length; c++) outputs[0][c].set(out);
    return true;
  }
}

registerProcessor("capture", Capture);
registerProcessor("player", Player);
