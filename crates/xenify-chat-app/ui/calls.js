// Calls: sound and pictures through the Xenify server.
//
// This web view (WebKitGTK on Linux) has no WebRTC, but it has WebCodecs, a
// camera, a microphone and screen capture. So each frame is encoded here
// (VP8 pictures, Opus sound) and sent over a WebSocket; the server passes it
// to everyone else in the call, marked with who sent it (8 bytes).
//
// A frame: kind (1 sound, 2 camera, 3 screen) · key frame (1/0) · timestamp
// (8 bytes) · the encoded bytes.
//
// `listen(sender, samples)`, when set, hears every piece of sound decoded
// here, and your own microphone's (48 kHz mono): recording uses it.

const SOUND = 1, CAMERA = 2, SCREEN = 3;

export class Call {
  /// `info`: the server's CallInfo; `me`: your user id; `hooks.changed()`
  /// redraws, `hooks.error(text)` says what went wrong.
  constructor(info, me, hooks) {
    this.info = info;
    this.me = me;
    this.hooks = hooks;
    this.mic = true;
    this.cam = !!info.video;
    this.screen = false;
    this.people = new Map(); // sender id -> { cam: canvas, screen: canvas, talking: time }
    this.sharing = null; // who shares a screen (id), if anyone
    this.closed = false;
    // Counts for the test trace (XENIFY_CHAT_LOG).
    this.stats = { sent: {}, heard: {}, encoded: {}, decoded: {}, notes: [] };
    this.statTimer = setInterval(() => this.hooks.trace?.(`call ${JSON.stringify(this.stats)}`), 2000);
  }

  /// Joins: the relay, then sound (and the camera if it's a video call).
  async start(url, token) {
    const ws = url.replace(/^http/, "ws").replace(/\/$/, "") + `/v1/call/${this.info.id}/media`;
    // Signed in as on /v1/ws: the key goes as a subprotocol, not in the address.
    this.ws = new WebSocket(ws, ["xenify-chat", token]);
    this.ws.binaryType = "arraybuffer";
    this.ws.onmessage = (e) => { try { this.heard(e.data); } catch {} };
    this.ws.onclose = () => { if (!this.closed) this.hooks.error("The call's connection dropped."); };
    await new Promise((ok, fail) => { this.ws.onopen = ok; this.ws.onerror = () => fail(new Error("Couldn't reach the call.")); });
    // The camera first (one request with the microphone: one prompt), then
    // sound.
    let both = null;
    if (this.cam) {
      both = await navigator.mediaDevices.getUserMedia({ audio: true, video: { width: 640, height: 360 } }).catch((e) => {
        this.stats.notes.push(`both: ${e.name} ${e.message}`);
        return null;
      });
      if (both) this.camOut = this.pictures(new MediaStream(both.getVideoTracks()), CAMERA, { width: 640, height: 360, fps: 15, bitrate: 500_000 });
    }
    this.ctx = new AudioContext({ sampleRate: 48000 });
    await this.ctx.audioWorklet.addModule("calls-worklet.js");
    this.player = new AudioWorkletNode(this.ctx, "player", { outputChannelCount: [2] });
    this.player.connect(this.ctx.destination);
    // No microphone (or not allowed): still in the call, listening.
    await this.startSound(both ? new MediaStream(both.getAudioTracks()) : null).catch((e) => {
      this.mic = false;
      this.noMic = true;
      this.hooks.error(`${deviceTrouble(e, "microphone")} You can listen, but others won't hear you.`);
    });
    if (this.cam && !both) await this.startCamera().catch((e) => { this.cam = false; this.hooks.error(`${deviceTrouble(e, "camera")} Others won't see you.`); });
    this.hooks.changed();
  }

  frame(kind, key, ts, bytes) {
    const out = new Uint8Array(10 + bytes.byteLength);
    out[0] = kind;
    out[1] = key ? 1 : 0;
    new DataView(out.buffer).setFloat64(2, ts);
    out.set(new Uint8Array(bytes), 10);
    if (this.ws?.readyState === 1 && this.ws.bufferedAmount < 4 << 20) {
      this.ws.send(out);
      this.stats.sent[kind] = (this.stats.sent[kind] || 0) + 1;
    }
  }

  // ---- Sending ----

  async startSound(given) {
    this.micStream = this.hooks.testAudio
      ? this.testStream(this.hooks.testAudio)
      : given || (await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } }));
    this.soundEnc = new AudioEncoder({
      output: (chunk) => {
        const b = new Uint8Array(chunk.byteLength);
        chunk.copyTo(b);
        this.frame(SOUND, true, chunk.timestamp, b);
      },
      error: (e) => this.hooks.error(`Sound: ${e.message || e}`),
    });
    this.soundEnc.configure({ codec: "opus", sampleRate: 48000, numberOfChannels: 1, bitrate: 32000 });
    const src = this.ctx.createMediaStreamSource(this.micStream);
    this.capture = new AudioWorkletNode(this.ctx, "capture");
    let ts = 0;
    this.capture.port.onmessage = (e) => {
      if (!this.mic || this.soundEnc.state !== "configured") return;
      this.listen?.(this.me, e.data);
      const data = new AudioData({ format: "f32-planar", sampleRate: 48000, numberOfFrames: e.data.length, numberOfChannels: 1, timestamp: ts, data: e.data });
      ts += 20000;
      this.soundEnc.encode(data);
      data.close();
    };
    src.connect(this.capture);
    // (The capture node must reach the output to run; it adds nothing to it.)
    const silent = this.ctx.createGain();
    silent.gain.value = 0;
    this.capture.connect(silent).connect(this.ctx.destination);
  }

  /// Tests (`XENIFY_VOICE_TEST_WAV`): a recording, over and over, as the
  /// microphone. `samples`: base64 of 16 kHz 16-bit sound.
  testStream(samples) {
    const s = atob(samples);
    const n = s.length >> 1;
    const buf = this.ctx.createBuffer(1, n, 16000);
    const ch = buf.getChannelData(0);
    for (let i = 0; i < n; i++) ch[i] = ((s.charCodeAt(2 * i) | (s.charCodeAt(2 * i + 1) << 8)) << 16 >> 16) / 32768;
    const src = this.ctx.createBufferSource();
    src.buffer = buf;
    src.loop = true;
    const out = this.ctx.createMediaStreamDestination();
    src.connect(out);
    src.start();
    return out.stream;
  }

  /// A picture source (camera or screen) → VP8 frames of `kind`.
  pictures(stream, kind, { width, height, fps, bitrate }) {
    // Your own picture is this very video. WebKitGTK only starts a camera's
    // video that's in the page and on screen: a small preview in the corner
    // until it plays, then the screens move it to your tile.
    const video = document.createElement("video");
    video.muted = true;
    video.autoplay = true;
    video.style.cssText = "position:fixed;left:12px;bottom:12px;width:160px;height:90px;z-index:100;border-radius:8px;pointer-events:none";
    document.body.append(video);
    // (Given the stream the moment the camera opens, WebKitGTK's video never
    // starts: give the camera a moment first.)
    setTimeout(() => { video.srcObject = stream; }, 400);
    video.addEventListener("playing", () => {
      video.className = `callcanvas ${kind === SCREEN ? "screen" : "cam"} own`;
      this.hooks.changed();
    }, { once: true });
    this.person(this.me)[kind === SCREEN ? "screen" : "cam"] = video;
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const g = canvas.getContext("2d");
    // The encoder is made once the picture plays (made sooner, it can hold
    // up WebKitGTK's camera).
    let enc = null;
    const makeEncoder = () => {
      enc = new VideoEncoder({
        output: (chunk) => {
          const b = new Uint8Array(chunk.byteLength);
          chunk.copyTo(b);
          this.frame(kind, chunk.type === "key", chunk.timestamp, b);
        },
        error: (e) => { this.stats.notes.push(`encode ${kind}: ${e.message || e}`); this.hooks.error(`Pictures: ${e.message || e}`); },
      });
      enc.configure({ codec: "vp8", width, height, bitrate, framerate: fps, latencyMode: "realtime" });
    };
    let n = 0;
    const t0 = performance.now();
    const timer = setInterval(() => {
      if (!enc && video.readyState >= 2) makeEncoder();
      if (!enc || video.readyState < 2 || enc.state !== "configured" || enc.encodeQueueSize > 2) {
        this.stats.waiting = `video ${kind}: ready ${video.readyState} ns ${video.networkState} paused ${video.paused} enc ${enc?.state}`;
        return;
      }
      // Fit the picture, keeping its shape.
      const vw = video.videoWidth || width, vh = video.videoHeight || height;
      const k = Math.min(width / vw, height / vh);
      g.fillStyle = "#000";
      g.fillRect(0, 0, width, height);
      g.drawImage(video, (width - vw * k) / 2, (height - vh * k) / 2, vw * k, vh * k);
      const frame = new VideoFrame(canvas, { timestamp: Math.round((performance.now() - t0) * 1000) });
      enc.encode(frame, { keyFrame: n % (fps * 2) === 0 || this.wantKey?.(kind) });
      this.stats.encoded[kind] = (this.stats.encoded[kind] || 0) + 1;
      frame.close();
      n++;
    }, 1000 / fps);
    return { stop: () => { clearInterval(timer); try { enc?.close(); } catch {} for (const t of stream.getTracks()) t.stop(); video.remove(); } };
  }

  async startCamera() {
    const s = await navigator.mediaDevices.getUserMedia({ video: { width: 640, height: 360 } });
    this.camOut = this.pictures(s, CAMERA, { width: 640, height: 360, fps: 15, bitrate: 500_000 });
  }

  async setCamera(on) {
    this.cam = on;
    if (on && !this.camOut) await this.startCamera().catch((e) => { this.cam = false; this.hooks.error(`${deviceTrouble(e, "camera")} Others won't see you.`); });
    if (!on && this.camOut) {
      this.camOut.stop();
      this.camOut = null;
      this.people.get(this.me)?.cam?.remove();
      if (this.people.get(this.me)) this.people.get(this.me).cam = null;
    }
    this.hooks.changed();
  }

  async setMic(on) {
    // No microphone at the start: try again (one may have been plugged in).
    if (on && this.noMic) {
      try {
        this.mic = true;
        await this.startSound(null);
        this.noMic = false;
      } catch (e) {
        this.mic = false;
        this.hooks.error(`${deviceTrouble(e, "microphone")} You can listen, but others won't hear you.`);
      }
      return this.hooks.changed();
    }
    this.mic = on;
    this.hooks.changed();
  }

  /// Shares the screen: the computer asks which screen or window.
  async setScreen(on) {
    if (on && !this.screenOut) {
      let s;
      try {
        s = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
      } catch (e) {
        // Cancelled in the computer's own picker: nothing to say.
        if (e?.name === "NotAllowedError" || e?.name === "AbortError") return;
        return this.hooks.error(screenTrouble(e));
      }
      this.screenOut = this.pictures(s, SCREEN, { width: 1280, height: 720, fps: 10, bitrate: 1_500_000 });
      s.getVideoTracks()[0].onended = () => this.setScreen(false);
      this.screen = true;
      this.sharing = this.me;
    } else if (!on && this.screenOut) {
      this.screenOut.stop();
      this.screenOut = null;
      this.screen = false;
      if (this.sharing === this.me) this.sharing = null;
      const p = this.people.get(this.me);
      if (p?.screen) { p.screen.remove(); p.screen = null; }
    }
    this.hooks.changed();
  }

  // ---- Receiving ----

  heard(buf) {
    if (buf.byteLength < 18) return;
    const sender = Number(new DataView(buf).getBigInt64(0));
    // (Never your own, should the server send it back.)
    if (sender === this.me) { this.stats.own = (this.stats.own || 0) + 1; return; }
    const body = new Uint8Array(buf, 8);
    const kind = body[0], key = body[1] === 1;
    this.stats.heard[kind] = (this.stats.heard[kind] || 0) + 1;
    const ts = new DataView(buf).getFloat64(10);
    const data = body.subarray(10);
    const p = this.person(sender);
    p.seen = performance.now();
    if (kind === SOUND) {
      // (Sound can arrive before this side's player is ready.)
      if (!this.player) return;
      if (!p.sound) {
        p.sound = new AudioDecoder({
          output: (ad) => {
            const f = new Float32Array(ad.numberOfFrames);
            ad.copyTo(f, { planeIndex: 0, format: "f32-planar" });
            ad.close();
            let peak = 0;
            for (let i = 0; i < f.length; i += 16) peak = Math.max(peak, Math.abs(f[i]));
            if (peak > 0.04) p.talking = performance.now();
            // (A recording here hears it too: given a copy, as the player takes this one.)
            if (this.listen) this.listen(sender, f.slice());
            this.player.port.postMessage({ sender, samples: f }, [f.buffer]);
          },
          error: () => {},
        });
        p.sound.configure({ codec: "opus", sampleRate: 48000, numberOfChannels: 1 });
      }
      p.sound.decode(new EncodedAudioChunk({ type: "key", timestamp: ts, data }));
    } else if (kind === CAMERA || kind === SCREEN) {
      const which = kind === CAMERA ? "camDec" : "screenDec";
      if (!p[which]) {
        if (!key) return; // wait for a whole picture
        const canvas = this.tile(sender, kind === CAMERA ? "cam" : "screen");
        p[which] = new VideoDecoder({
          output: (frame) => {
            this.stats.decoded[kind] = (this.stats.decoded[kind] || 0) + 1;
            if (canvas.width !== frame.displayWidth) { canvas.width = frame.displayWidth; canvas.height = frame.displayHeight; }
            canvas.getContext("2d").drawImage(frame, 0, 0);
            frame.close();
          },
          error: (e) => { this.stats.notes.push(`decode ${kind}: ${e.message || e}`); p[which] = null; },
        });
        p[which].configure({ codec: "vp8" });
        if (kind === SCREEN) { this.sharing = sender; this.hooks.changed(); }
        if (kind === CAMERA) this.hooks.changed();
      }
      p[which].decode(new EncodedVideoChunk({ type: key ? "key" : "delta", timestamp: ts, data }));
      p[kind === CAMERA ? "lastCam" : "lastScreen"] = performance.now();
    }
  }

  person(id) {
    let p = this.people.get(id);
    if (!p) this.people.set(id, (p = {}));
    return p;
  }

  /// The canvas someone's camera or screen is drawn on (kept across redraws:
  /// the screens put it back where its place is).
  tile(id, what, w = 640, h = 360) {
    const p = this.person(id);
    if (!p[what]) {
      const c = document.createElement("canvas");
      c.width = w;
      c.height = h;
      c.className = `callcanvas ${what}`;
      c.dataset.callTile = `${id}-${what}`;
      p[what] = c;
    }
    return p[what];
  }

  /// People who left: their decoders and pictures go.
  forget(ids) {
    for (const [id, p] of this.people) {
      if (id === this.me || ids.includes(id)) continue;
      for (const d of ["sound", "camDec", "screenDec"]) try { p[d]?.close(); } catch {}
      this.people.delete(id);
      if (this.sharing === id) this.sharing = null;
    }
  }

  /// Turns your microphone, camera and shared screen off (someone started
  /// recording).
  async allOff() {
    this.mic = false;
    if (this.cam) await this.setCamera(false);
    if (this.screen) await this.setScreen(false);
    this.hooks.changed();
  }

  /// Hangs up here.
  stop() {
    this.closed = true;
    clearInterval(this.statTimer);
    this.camOut?.stop();
    this.screenOut?.stop();
    for (const t of this.micStream?.getTracks() || []) t.stop();
    try { this.soundEnc?.close(); } catch {}
    this.forget([]);
    this.ctx?.close().catch(() => {});
    this.ws?.close();
  }
}

/// Why a microphone or camera didn't start, in plain words.
/** Why screen sharing didn't start, in words. */
function screenTrouble(e) {
  // Linux's web view finds screens through the desktop's screen-sharing
  // service; without it there's nothing to pick (it says "Invalid constraint").
  if (e?.name === "OverconstrainedError" || e?.name === "NotFoundError") {
    return /Linux/.test(navigator.userAgent)
      ? "Sharing didn't start: this computer's screen-sharing service isn't running. It needs xdg-desktop-portal and PipeWire, which most desktops have; on a server or in a container, they're missing."
      : "Sharing didn't start: this computer didn't offer a screen to share.";
  }
  if (e?.name === "NotReadableError") return "Sharing didn't start: the computer wouldn't hand over the screen.";
  return `Sharing didn't start (${e?.message || e}).`;
}

function deviceTrouble(e, what) {
  if (e?.name === "NotFoundError" || e?.name === "OverconstrainedError") return `No ${what} was found on this computer.`;
  if (e?.name === "NotAllowedError") return `This computer didn't allow the ${what}.`;
  if (e?.name === "NotReadableError") return `The ${what} is busy in another app.`;
  return `The ${what} didn't start (${e?.message || e}).`;
}
