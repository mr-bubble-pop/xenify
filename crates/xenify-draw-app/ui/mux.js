// Video files from WebCodecs chunks: MP4 (H.264, with AAC or Opus sound)
// and WebM (VP9 or VP8, with Opus sound). Chunks are { t: µs, d: µs, key, b }.

const concat = (parts) => {
  const out = new Uint8Array(parts.reduce((s, p) => s + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
};
const u8 = (...v) => new Uint8Array(v);
const be = (n, bytes) => { const out = new Uint8Array(bytes); let v = BigInt(Math.round(n)); for (let i = bytes - 1; i >= 0; i--) { out[i] = Number(v & 0xFFn); v >>= 8n; } return out; };
const u16 = (n) => be(n, 2), u32 = (n) => be(n, 4);
const ascii = (s) => new TextEncoder().encode(s);

/// The OpusHead both WebM and MP4 (as dOps, big-endian) describe Opus with.
function opusHead(preSkip, le = true) {
  const d = new DataView(new ArrayBuffer(19));
  ascii('OpusHead').forEach((c, i) => d.setUint8(i, c));
  d.setUint8(8, 1); d.setUint8(9, 2); d.setUint16(10, preSkip, le); d.setUint32(12, 48000, le); d.setInt16(16, 0, le); d.setUint8(18, 0);
  return new Uint8Array(d.buffer);
}

// ----- MP4 -----

const box = (type, ...kids) => { const body = concat(kids.flat()); return concat([u32(8 + body.length), ascii(type), body]); };
const full = (type, version, flags, ...kids) => box(type, u8(version), be(flags, 3), ...kids);
const matrix = () => concat([u32(0x00010000), u32(0), u32(0), u32(0), u32(0x00010000), u32(0), u32(0), u32(0), u32(0x40000000)]);

/// Durations as stts runs.
function stts(durs) {
  const runs = [];
  for (const d of durs) { const last = runs[runs.length - 1]; if (last && last[1] === d) last[0]++; else runs.push([1, d]); }
  return full('stts', 0, 0, u32(runs.length), runs.map(([n, d]) => concat([u32(n), u32(d)])));
}

/// AAC's esds: the decoder config wrapped in MPEG-4 descriptors.
function esds(asc) {
  const desc = (tag, body) => concat([u8(tag, 0x80, 0x80, 0x80, body.length), body]);
  const dcd = desc(0x04, concat([u8(0x40, 0x15), be(0, 3), u32(128000), u32(128000), desc(0x05, asc)]));
  return full('esds', 0, 0, desc(0x03, concat([u16(1), u8(0), dcd, desc(0x06, u8(0x02))])));
}

export function mp4({ width, height, fps, video, videoConfig, audio, audioCodec, audioConfig, preSkip = 312 }) {
  const vScale = 90000, aScale = 48000;
  const vDurs = video.map((c) => Math.max(1, Math.round((c.d || 1e6 / fps) * vScale / 1e6)));
  const aDurs = (audio || []).map((c) => Math.max(1, Math.round(c.d * aScale / 1e6)));
  const vTotal = vDurs.reduce((a, b) => a + b, 0), aTotal = aDurs.reduce((a, b) => a + b, 0);
  const durMs = Math.max(vTotal / vScale, aTotal / aScale) * 1000;
  const ftyp = box('ftyp', ascii('isom'), u32(512), ascii('isom'), ascii('iso2'), ascii('avc1'), ascii('mp41'));
  const vData = concat(video.map((c) => c.b)), aData = concat((audio || []).map((c) => c.b));
  const mdatHead = 8;
  const build = (vOff, aOff) => {
    const vTrak = box('trak',
      full('tkhd', 0, 3, u32(0), u32(0), u32(1), u32(0), u32(durMs), u32(0), u32(0), u16(0), u16(0), u16(0), u16(0), matrix(), u32(width * 65536), u32(height * 65536)),
      box('mdia',
        full('mdhd', 0, 0, u32(0), u32(0), u32(vScale), u32(vTotal), u16(0x55C4), u16(0)),
        full('hdlr', 0, 0, u32(0), ascii('vide'), u32(0), u32(0), u32(0), ascii('Video\0')),
        box('minf', full('vmhd', 0, 1, u16(0), u16(0), u16(0), u16(0)),
          box('dinf', full('dref', 0, 0, u32(1), full('url ', 0, 1))),
          box('stbl',
            full('stsd', 0, 0, u32(1), box('avc1', new Uint8Array(6), u16(1), new Uint8Array(16), u16(width), u16(height), u32(0x00480000), u32(0x00480000), u32(0), u16(1), new Uint8Array(32), u16(0x18), u16(0xFFFF), box('avcC', videoConfig))),
            stts(vDurs),
            full('stss', 0, 0, u32(video.filter((c) => c.key).length), video.map((c, i) => (c.key ? u32(i + 1) : null)).filter(Boolean)),
            full('stsc', 0, 0, u32(1), u32(1), u32(video.length), u32(1)),
            full('stsz', 0, 0, u32(0), u32(video.length), video.map((c) => u32(c.b.length))),
            full('stco', 0, 0, u32(1), u32(vOff))))));
    const aTrak = audio?.length ? box('trak',
      full('tkhd', 0, 3, u32(0), u32(0), u32(2), u32(0), u32(durMs), u32(0), u32(0), u16(0), u16(1), u16(0x0100), u16(0), matrix(), u32(0), u32(0)),
      box('mdia',
        full('mdhd', 0, 0, u32(0), u32(0), u32(aScale), u32(aTotal), u16(0x55C4), u16(0)),
        full('hdlr', 0, 0, u32(0), ascii('soun'), u32(0), u32(0), u32(0), ascii('Sound\0')),
        box('minf', full('smhd', 0, 0, u16(0), u16(0)),
          box('dinf', full('dref', 0, 0, u32(1), full('url ', 0, 1))),
          box('stbl',
            full('stsd', 0, 0, u32(1), audioCodec === 'aac'
              ? box('mp4a', new Uint8Array(6), u16(1), new Uint8Array(8), u16(2), u16(16), u16(0), u16(0), u32(aScale * 65536), esds(audioConfig))
              : box('Opus', new Uint8Array(6), u16(1), new Uint8Array(8), u16(2), u16(16), u16(0), u16(0), u32(aScale * 65536), box('dOps', u8(0, 2), u16(preSkip), u32(48000), u16(0), u8(0)))),
            stts(aDurs),
            full('stsc', 0, 0, u32(1), u32(1), u32(audio.length), u32(1)),
            full('stsz', 0, 0, u32(0), u32(audio.length), audio.map((c) => u32(c.b.length))),
            full('stco', 0, 0, u32(1), u32(aOff)))))) : null;
    const mvhd = full('mvhd', 0, 0, u32(0), u32(0), u32(1000), u32(durMs), u32(0x00010000), u16(0x0100), u16(0), u32(0), u32(0), matrix(), new Uint8Array(24), u32(3));
    return box('moov', mvhd, vTrak, aTrak ? [aTrak] : []);
  };
  // The moov goes first (fast start): its size doesn't depend on the offsets' values.
  const moovSize = build(0, 0).length;
  const vOff = ftyp.length + moovSize + mdatHead;
  const moov = build(vOff, vOff + vData.length);
  return concat([ftyp, moov, u32(mdatHead + vData.length + aData.length), ascii('mdat'), vData, aData]);
}

// ----- WebM (Matroska) -----

function vint(n) {
  const b = new Uint8Array(8);
  b[0] = 0x01;
  let v = BigInt(n);
  for (let i = 7; i >= 1; i--) { b[i] = Number(v & 0xFFn); v >>= 8n; }
  return b;
}
function el(id, ...data) {
  const idb = [];
  for (let v = id; v > 0; v = Math.floor(v / 256)) idb.unshift(v & 0xFF);
  const body = concat(data.flat());
  return concat([new Uint8Array(idb), vint(body.length), body]);
}
const uint = (n) => { const out = []; let v = n; do { out.unshift(v & 0xFF); v = Math.floor(v / 256); } while (v > 0); return new Uint8Array(out); };
const f64 = (x) => { const d = new DataView(new ArrayBuffer(8)); d.setFloat64(0, x); return new Uint8Array(d.buffer); };

export function webm({ width, height, video, codec, audio, preSkip = 312 }) {
  const durationMs = Math.max(...video.map((c) => (c.t + (c.d || 0)) / 1000), ...(audio || []).map((c) => (c.t + c.d) / 1000), 0);
  const header = el(0x1A45DFA3, el(0x4286, uint(1)), el(0x42F7, uint(1)), el(0x42F2, uint(4)), el(0x42F3, uint(8)), el(0x4282, ascii('webm')), el(0x4287, uint(2)), el(0x4285, uint(2)));
  const info = el(0x1549A966, el(0x2AD7B1, uint(1000000)), el(0x4D80, ascii('Xenify Draw')), el(0x5741, ascii('Xenify Draw')), el(0x4489, f64(durationMs)));
  const tracks = [el(0xAE, el(0xD7, uint(1)), el(0x73C5, uint(1)), el(0x83, uint(1)), el(0x86, ascii(codec)), el(0xE0, el(0xB0, uint(width)), el(0xBA, uint(height))))];
  if (audio?.length) {
    tracks.push(el(0xAE, el(0xD7, uint(2)), el(0x73C5, uint(2)), el(0x83, uint(2)), el(0x86, ascii('A_OPUS')), el(0x63A2, opusHead(preSkip)),
      el(0x56AA, uint(Math.round(preSkip / 48000 * 1e9))), el(0xE1, el(0xB5, f64(48000)), el(0x9F, uint(2)))));
  }
  const blocks = [...video.map((c) => ({ ...c, track: 1 })), ...(audio || []).map((c) => ({ ...c, track: 2, key: true }))].sort((a, b) => a.t - b.t || a.track - b.track);
  const clusters = [];
  let cluster = null;
  const close = () => { if (cluster) clusters.push(el(0x1F43B675, el(0xE7, uint(cluster.t)), ...cluster.blocks)); };
  for (const c of blocks) {
    const t = Math.round(c.t / 1000);
    if (!cluster || (c.track === 1 && c.key) || t - cluster.t > 30000) { close(); cluster = { t, blocks: [] }; }
    const rel = t - cluster.t;
    cluster.blocks.push(el(0xA3, u8(0x80 | c.track, (rel >> 8) & 0xFF, rel & 0xFF, c.key ? 0x80 : 0x00), c.b));
  }
  close();
  return concat([header, el(0x18538067, info, el(0x1654AE6B, ...tracks), ...clusters)]);
}

/// Encodes 48 kHz stereo (interleaved) with WebCodecs: chunks, the codec used ('aac' or 'opus') and its config.
export async function encodeAudio(pcm, prefer) {
  const frames = pcm.length / 2;
  if (!frames || typeof AudioEncoder !== 'function') return null;
  const tryCodec = async (codec) => (await AudioEncoder.isConfigSupported({ codec, sampleRate: 48000, numberOfChannels: 2, bitrate: 160000 }).catch(() => ({ supported: false }))).supported;
  const codec = prefer === 'aac' && (await tryCodec('mp4a.40.2')) ? 'mp4a.40.2' : (await tryCodec('opus')) ? 'opus' : null;
  if (!codec) return null;
  const chunks = [];
  let config = null;
  const enc = new AudioEncoder({ output: (c, meta) => { const b = new Uint8Array(c.byteLength); c.copyTo(b); chunks.push({ t: c.timestamp, d: c.duration || 20000, key: true, b }); if (meta?.decoderConfig?.description) config = new Uint8Array(meta.decoderConfig.description); }, error: () => {} });
  enc.configure({ codec, sampleRate: 48000, numberOfChannels: 2, bitrate: 160000 });
  const step = 4800;
  for (let i = 0; i < frames; i += step) {
    const n = Math.min(step, frames - i);
    const data = pcm.slice(i * 2, (i + n) * 2);
    const ad = new AudioData({ format: 'f32', sampleRate: 48000, numberOfFrames: n, numberOfChannels: 2, timestamp: Math.round((i / 48000) * 1e6), data });
    enc.encode(ad);
    ad.close();
  }
  await enc.flush();
  enc.close();
  return { codec: codec === 'opus' ? 'opus' : 'aac', chunks, config };
}
