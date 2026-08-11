// Browser glue for the qr-demo WebAssembly module.
//
// Camera scanning: a frame is drawn to an offscreen canvas and its raw RGBA
// handed to a `Scanner`, which is stateful across frames — it carries qr-video's
// region-of-interest tracking, full-scan cadence and temporal de-duplication,
// and returns first/repeat/lost events rather than a bare detection list. The
// RGBA -> luma reduction happens inside wasm; see `Scanner.scan_frame_rgba`.
// File scanning: one-shot, so it uses the stateless `scan`, which also reports
// the symbol version the streaming path cannot.
// Generating: `generate` returns an SVG document string, or a message starting
// with "ERROR:".

import init, { scan, generate, Scanner } from './pkg/qr_demo.js';

const el = (id) => document.getElementById(id);

const video = el('video');
const preview = el('preview');
const overlay = el('overlay');
const stageHint = el('stage-hint');
const scanStatus = el('scan-status');
const results = el('results');
const startCam = el('start-cam');
const stopCam = el('stop-cam');
const fileInput = el('file-input');

const genText = el('gen-text');
const genEc = el('gen-ec');
const genMicro = el('gen-micro');
const genOut = el('gen-out');
const genError = el('gen-error');
const download = el('download');

// Longest edge the detector sees. More pixels per module is the single biggest
// low-light win — sensor noise averages out over a larger module area — and the
// detector is fast enough to afford it: ~1.5 ms on a 1080p frame.
const MAX_EDGE = 1440;
// ~15 scans per second; the rAF loop simply skips frames in between.
const SCAN_INTERVAL_MS = 66;
// How many distinct payloads to keep in the decoded list.
const MAX_RESULTS = 8;
// How long an overlay box survives without a fresh sighting. The Micro QR pass
// runs on a slower cadence than the QR one and an ROI scan can miss a frame, so
// clearing the overlay on every frame with no event would make the box blink.
const OVERLAY_TTL_MS = 300;
// Frames kept for the rolling median scan time in the status line.
const TIMING_WINDOW = 20;

const work = document.createElement('canvas');
const workCtx = work.getContext('2d', { willReadFrequently: true });
const overlayCtx = overlay.getContext('2d');

let stream = null;
let rafId = 0;
let lastScan = 0;
let scanner = null;
/** Payload key -> latest sighting, for the overlay. Camera path only. */
const live = new Map();
/** Recent wasm scan durations, milliseconds. */
const timings = [];
let frameSize = null;
const seen = new Map();

// ---------------------------------------------------------------------------
// wasm boot
// ---------------------------------------------------------------------------

const ready = init().then(
  () => true,
  (err) => {
    setStatus(`Could not load the WebAssembly module: ${err}. Serve this folder over HTTP — opening index.html from the filesystem will not work.`, true);
    return false;
  },
);

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

function setStatus(msg, isError = false) {
  scanStatus.textContent = msg;
  scanStatus.classList.toggle('error', isError);
}

/**
 * Draws `source` into the work canvas, bounded to `MAX_EDGE` on its longer
 * side, and returns the raw RGBA plane. The grayscale reduction is left to
 * wasm — it is roughly twice as fast there, even counting the wider copy.
 */
function grabFrame(source, sw, sh) {
  const factor = Math.min(1, MAX_EDGE / Math.max(sw, sh));
  const w = Math.max(1, Math.round(sw * factor));
  const h = Math.max(1, Math.round(sh * factor));
  if (work.width !== w || work.height !== h) {
    work.width = w;
    work.height = h;
  }
  workCtx.drawImage(source, 0, 0, w, h);
  const { data } = workCtx.getImageData(0, 0, w, h);
  // getImageData hands back a Uint8ClampedArray; wasm-bindgen wants Uint8Array
  // over the same buffer, which is a view, not a copy.
  return { rgba: new Uint8Array(data.buffer, data.byteOffset, data.length), w, h };
}

/** Median of a short unsorted array of numbers; 0 when empty. */
function median(values) {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[sorted.length >> 1];
}

function recordTiming(ms) {
  timings.push(ms);
  if (timings.length > TIMING_WINDOW) timings.shift();
}

/** `1440×810 · 3.1 ms` — the two numbers that explain a bad scanning session. */
function perfSuffix() {
  if (!frameSize) return '';
  const ms = median(timings);
  return ` — ${frameSize.w}×${frameSize.h}, ${ms.toFixed(1)} ms/scan`;
}

/**
 * Maps detector coordinates (work-canvas pixels) onto the overlay, accounting
 * for the letterboxing `object-fit: contain` introduces when the source aspect
 * ratio differs from the element box.
 */
function overlayMapper(sw, sh) {
  const ew = overlay.clientWidth;
  const eh = overlay.clientHeight;
  if (overlay.width !== ew || overlay.height !== eh) {
    overlay.width = ew;
    overlay.height = eh;
  }
  const s = Math.min(ew / sw, eh / sh);
  const dw = sw * s;
  const dh = sh * s;
  const ox = (ew - dw) / 2;
  const oy = (eh - dh) / 2;
  return (x, y) => [ox + (x / sw) * dw, oy + (y / sh) * dh];
}

/** `boxes` are `{ corners, kind, version, unreadable }` in detector pixels. */
function drawOverlay(boxes, sw, sh) {
  const map = overlayMapper(sw, sh);
  overlayCtx.clearRect(0, 0, overlay.width, overlay.height);
  overlayCtx.lineWidth = 3;
  overlayCtx.lineJoin = 'round';
  overlayCtx.font = '13px system-ui, sans-serif';

  for (const d of boxes) {
    overlayCtx.strokeStyle = d.unreadable ? '#ffb020' : '#39d98a';
    overlayCtx.fillStyle = overlayCtx.strokeStyle;

    overlayCtx.beginPath();
    d.corners.forEach(([x, y], i) => {
      const [px, py] = map(x, y);
      if (i === 0) overlayCtx.moveTo(px, py);
      else overlayCtx.lineTo(px, py);
    });
    overlayCtx.closePath();
    overlayCtx.stroke();

    const [lx, ly] = map(d.corners[0][0], d.corners[0][1]);
    const label = symbolLabel(d);
    overlayCtx.fillText(d.unreadable ? `${label} — unreadable` : label, lx, Math.max(12, ly - 6));
  }
}

/**
 * "QR v7" / "Micro M2" / plain "QR" when the version is unknown. The streaming
 * path has no version to report: qr-video's observations carry the payload but
 * not the symbol geometry.
 */
function symbolLabel(d) {
  if (d.kind === 'micro') return d.version ? `Micro M${d.version}` : 'Micro';
  return d.version ? `QR v${d.version}` : 'QR';
}

/**
 * Folds decoded payloads into the distinct-results list. Entries are
 * `{ kind, version, text, bytes_hex }`; a payload already listed only refreshes
 * its timestamp, and only once a second, to keep re-renders off the hot path.
 */
function recordResults(entries) {
  const now = Date.now();
  let changed = false;
  for (const d of entries) {
    const key = `${d.kind}:${d.text ?? d.bytes_hex}`;
    const existing = seen.get(key);
    if (!existing) {
      seen.set(key, { ...d, at: now });
      changed = true;
    } else if (now - existing.at > 1000) {
      existing.at = now;
      changed = true;
    }
  }
  if (changed) renderResults();
}

function renderResults() {
  const entries = [...seen.values()].sort((a, b) => b.at - a.at).slice(0, MAX_RESULTS);
  // Anything trimmed off the end is gone for good; drop it so the map cannot
  // grow without bound over a long scanning session.
  if (seen.size > MAX_RESULTS) {
    const keep = new Set(entries.map((e) => `${e.kind}:${e.text ?? e.bytes_hex}`));
    for (const key of seen.keys()) if (!keep.has(key)) seen.delete(key);
  }

  results.replaceChildren();
  if (entries.length === 0) {
    const li = document.createElement('li');
    li.className = 'empty';
    li.textContent = 'Nothing scanned yet.';
    results.append(li);
    return;
  }

  for (const e of entries) {
    const li = document.createElement('li');

    const meta = document.createElement('div');
    meta.className = 'meta';
    const tag = document.createElement('span');
    tag.className = 'tag';
    tag.textContent = symbolLabel(e);
    const time = document.createElement('span');
    time.textContent = new Date(e.at).toLocaleTimeString();
    meta.append(tag, time);

    const payload = document.createElement('div');
    payload.className = 'payload';
    payload.textContent = e.text ?? `<binary> ${e.bytes_hex}`;

    li.append(meta, payload);
    results.append(li);
  }
}

/**
 * Folds one frame's events into the overlay map. `first` is a payload the
 * detector had not been tracking, and the only kind worth listing; `repeat`
 * only refreshes the box; `lost` retires it. Boxes not refreshed within
 * `OVERLAY_TTL_MS` are dropped, which covers the frames a symbol is simply not
 * reported on.
 */
function applyEvents(events, now) {
  const fresh = [];
  for (const e of events) {
    // Keys are qualified by symbology: a QR and a Micro carrying the same
    // payload hash to the same key, and one's `lost` must not retire the
    // other's box.
    const boxKey = `${e.kind}:${e.key}`;
    if (e.event === 'lost') {
      live.delete(boxKey);
      continue;
    }
    live.set(boxKey, {
      kind: e.kind,
      version: e.version,
      corners: e.corners,
      unreadable: e.unreadable,
      at: now,
    });
    if (e.event === 'first' && !e.unreadable) fresh.push(e);
  }
  for (const [key, box] of live) {
    if (now - box.at > OVERLAY_TTL_MS) live.delete(key);
  }
  if (fresh.length > 0) recordResults(fresh);
}

function cameraStatus() {
  const boxes = [...live.values()];
  if (boxes.length === 0) return 'No symbol in frame.';
  const unreadable = boxes.filter((b) => b.unreadable).length;
  const decoded = boxes.length - unreadable;
  if (unreadable === 0) return `${decoded} symbol${decoded === 1 ? '' : 's'} in view.`;
  return `${decoded} decoded, ${unreadable} found but unreadable — move closer, hold steady, or add light.`;
}

function loop(now) {
  rafId = requestAnimationFrame(loop);
  if (now - lastScan < SCAN_INTERVAL_MS) return;
  lastScan = now;
  if (video.readyState < 2 || video.videoWidth === 0) return;

  const { rgba, w, h } = grabFrame(video, video.videoWidth, video.videoHeight);
  frameSize = { w, h };

  let events;
  const started = performance.now();
  try {
    events = JSON.parse(scanner.scan_frame_rgba(rgba, w, h, now));
  } catch (err) {
    // A throw here is the wasm module failing, not a bad frame; scanning on
    // would repeat the same failure fifteen times a second.
    stopCamera();
    setStatus(`Scanning stopped — the detector failed: ${err}`, true);
    return;
  }
  recordTiming(performance.now() - started);

  applyEvents(events, now);
  drawOverlay([...live.values()], w, h);
  setStatus(cameraStatus() + perfSuffix());
}

/**
 * Asks the track for continuous autofocus and auto-exposure. Both are optional
 * MediaTrackCapabilities, absent on desktop browsers and on Safari; a camera
 * that does not advertise them is left on its defaults.
 */
async function tuneTrack(track) {
  const caps = track.getCapabilities?.();
  if (!caps) return;
  const advanced = [];
  if (caps.focusMode?.includes('continuous')) advanced.push({ focusMode: 'continuous' });
  if (caps.exposureMode?.includes('continuous')) advanced.push({ exposureMode: 'continuous' });
  if (advanced.length === 0) return;
  try {
    await track.applyConstraints({ advanced });
  } catch {
    // Purely an enhancement — a rejected constraint set costs nothing.
  }
}

async function startCamera() {
  if (!navigator.mediaDevices?.getUserMedia) {
    setStatus('This browser exposes no camera API. Use the image file input instead.', true);
    return;
  }
  if (!(await ready)) return;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      // 1080p ideal: the detector has budget for it and the extra pixels per
      // module are what carry a symbol through poor light. `ideal` so a camera
      // that cannot do it still opens at whatever it has.
      video: {
        facingMode: { ideal: 'environment' },
        width: { ideal: 1920 },
        height: { ideal: 1080 },
      },
      audio: false,
    });
  } catch (err) {
    setStatus(`Camera unavailable: ${err.name === 'NotAllowedError' ? 'permission denied' : err.message}. The page must be served from localhost or over HTTPS.`, true);
    return;
  }
  const [track] = stream.getVideoTracks();
  if (track) await tuneTrack(track);

  preview.hidden = true;
  video.hidden = false;
  stageHint.hidden = true;
  video.srcObject = stream;
  await video.play();
  startCam.disabled = true;
  stopCam.disabled = false;

  scanner ??= new Scanner();
  scanner.reset();
  live.clear();
  timings.length = 0;
  frameSize = null;
  setStatus('Scanning…');
  lastScan = 0;
  rafId = requestAnimationFrame(loop);
}

function stopCamera() {
  if (rafId) cancelAnimationFrame(rafId);
  rafId = 0;
  if (stream) {
    for (const track of stream.getTracks()) track.stop();
    stream = null;
  }
  video.srcObject = null;
  video.hidden = true;
  startCam.disabled = false;
  stopCam.disabled = true;
  overlayCtx.clearRect(0, 0, overlay.width, overlay.height);
  live.clear();
  scanner?.reset();
  if (preview.hidden) stageHint.hidden = false;
  setStatus('Camera stopped.');
}

/**
 * RGBA to luma for the one-shot file path, which goes through the stateless
 * `scan`. Doing it here rather than adding a second wasm entry point keeps the
 * exported surface to the shape the camera actually uses; this runs once per
 * picked file, so its cost is irrelevant.
 */
function rgbaToLuma(rgba) {
  const luma = new Uint8Array(rgba.length / 4);
  for (let i = 0, p = 0; i < luma.length; i++, p += 4) {
    // Integer approximation of 0.299R + 0.587G + 0.114B.
    luma[i] = (77 * rgba[p] + 150 * rgba[p + 1] + 29 * rgba[p + 2]) >> 8;
  }
  return luma;
}

async function runFileScan(source, sw, sh) {
  if (!(await ready)) return;
  const { rgba, w, h } = grabFrame(source, sw, sh);
  frameSize = { w, h };
  timings.length = 0;

  let detections;
  const started = performance.now();
  try {
    detections = JSON.parse(scan(rgbaToLuma(rgba), w, h));
  } catch (err) {
    setStatus(`Scan failed: ${err}`, true);
    return;
  }
  recordTiming(performance.now() - started);

  drawOverlay(
    detections.map((d) => ({
      kind: d.kind,
      version: d.version,
      corners: d.corners,
      unreadable: d.error !== null,
    })),
    w,
    h,
  );
  recordResults(detections.filter((d) => d.error === null));

  const unreadable = detections.filter((d) => d.error !== null).length;
  const decoded = detections.length - unreadable;
  if (detections.length === 0) setStatus(`No symbol found.${perfSuffix()}`);
  else if (unreadable === 0) setStatus(`${decoded} symbol${decoded === 1 ? '' : 's'} decoded.${perfSuffix()}`);
  else setStatus(`${decoded} decoded, ${unreadable} found but unreadable.${perfSuffix()}`);
}

function scanFile(file) {
  stopCamera();
  const url = URL.createObjectURL(file);
  const img = new Image();
  img.onload = () => {
    preview.width = img.naturalWidth;
    preview.height = img.naturalHeight;
    preview.getContext('2d').drawImage(img, 0, 0);
    preview.hidden = false;
    stageHint.hidden = true;
    URL.revokeObjectURL(url);
    // Let layout settle so the overlay picks up the preview's final box.
    requestAnimationFrame(() => runFileScan(preview, preview.width, preview.height));
  };
  img.onerror = () => {
    URL.revokeObjectURL(url);
    setStatus('That file could not be read as an image.', true);
  };
  img.src = url;
}

startCam.addEventListener('click', startCamera);
stopCam.addEventListener('click', stopCamera);
fileInput.addEventListener('change', () => {
  const file = fileInput.files?.[0];
  if (file) scanFile(file);
  fileInput.value = '';
});
window.addEventListener('resize', () => {
  overlayCtx.clearRect(0, 0, overlay.width, overlay.height);
});

// ---------------------------------------------------------------------------
// Generating
// ---------------------------------------------------------------------------

let downloadUrl = null;
let downloadPngUrl = null;
let genTimer = 0;
let genSeq = 0;

const downloadPng = el('download-png');
const copyPng = el('copy-png');

/// Longest side of exported PNGs, in pixels. The SVG is crisp at any size;
/// 1024 keeps the raster scannable without bloating the clipboard.
const PNG_SIZE = 1024;

/** Rasterizes the current SVG markup to a PNG blob via an offscreen canvas. */
function svgToPngBlob(svg) {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      const canvas = document.createElement('canvas');
      canvas.width = PNG_SIZE;
      canvas.height = PNG_SIZE;
      const ctx = canvas.getContext('2d');
      // The renderer's quiet zone is part of the SVG, but a solid backdrop
      // keeps the PNG readable on dark surfaces even if that ever changes.
      ctx.fillStyle = '#fff';
      ctx.fillRect(0, 0, PNG_SIZE, PNG_SIZE);
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(img, 0, 0, PNG_SIZE, PNG_SIZE);
      canvas.toBlob(
        (blob) => (blob ? resolve(blob) : reject(new Error('PNG encode failed'))),
        'image/png',
      );
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error('SVG rasterize failed'));
    };
    img.src = url;
  });
}

function clearPngExports() {
  downloadPng.hidden = true;
  copyPng.hidden = true;
  if (downloadPngUrl) {
    URL.revokeObjectURL(downloadPngUrl);
    downloadPngUrl = null;
  }
}

/** Micro QR has no level H; only Micro QR has the M1 detection-only selection. */
function syncEcOptions() {
  const micro = genMicro.checked;
  const optH = genEc.querySelector('option[value="H"]');
  const optD = genEc.querySelector('option[value="D"]');
  optH.disabled = micro;
  optH.hidden = micro;
  optD.disabled = !micro;
  optD.hidden = !micro;
  if (micro && genEc.value === 'H') genEc.value = 'Q';
  if (!micro && genEc.value === 'D') genEc.value = 'M';
}

async function regenerate() {
  if (!(await ready)) return;

  // An empty box is the neutral starting state, not a user mistake: show the
  // placeholder frame and no error.
  if (genText.value.length === 0) {
    genOut.replaceChildren();
    genOut.hidden = false;
    genError.hidden = true;
    download.hidden = true;
    clearPngExports();
    if (downloadUrl) {
      URL.revokeObjectURL(downloadUrl);
      downloadUrl = null;
    }
    return;
  }

  const svg = generate(genText.value, genEc.value, genMicro.checked);

  if (svg.startsWith('ERROR:')) {
    genOut.replaceChildren();
    genOut.hidden = true;
    genError.textContent = svg.slice('ERROR:'.length).trim();
    genError.hidden = false;
    download.hidden = true;
    clearPngExports();
    if (downloadUrl) {
      URL.revokeObjectURL(downloadUrl);
      downloadUrl = null;
    }
    return;
  }

  genError.hidden = true;
  genOut.hidden = false;
  // The SVG comes from the renderer in this page's own wasm module, not from
  // user input, so injecting it as markup is safe.
  genOut.innerHTML = svg;

  if (downloadUrl) URL.revokeObjectURL(downloadUrl);
  downloadUrl = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
  download.href = downloadUrl;
  const stem = genMicro.checked ? 'microqr' : 'qrcode';
  download.download = `${stem}.svg`;
  download.hidden = false;

  // PNG export: rasterized once per (debounced) regeneration. The sequence
  // number drops a slow rasterize that finishes after a newer regeneration.
  // (Comparing genOut.innerHTML to the SVG string does NOT work: setting
  // innerHTML normalizes the markup, so they never compare equal.)
  const seq = ++genSeq;
  clearPngExports();
  try {
    const blob = await svgToPngBlob(svg);
    if (seq !== genSeq) return; // regenerated meanwhile
    downloadPngUrl = URL.createObjectURL(blob);
    downloadPng.href = downloadPngUrl;
    downloadPng.download = `${stem}.png`;
    downloadPng.hidden = false;
    // Clipboard needs a secure context (https or localhost) and ClipboardItem.
    if (navigator.clipboard && window.ClipboardItem) {
      copyPng.hidden = false;
      copyPng.dataset.svg = svg;
    }
  } catch {
    // PNG export is a convenience; the SVG link above still works.
  }
}

copyPng.addEventListener('click', async () => {
  const svg = copyPng.dataset.svg;
  if (!svg) return;
  const original = 'Copy PNG';
  try {
    // Rasterize at click time: Safari requires the ClipboardItem promise to be
    // created inside the user gesture, and a fresh blob avoids holding one.
    const item = new ClipboardItem({ 'image/png': svgToPngBlob(svg) });
    await navigator.clipboard.write([item]);
    copyPng.textContent = 'Copied!';
  } catch {
    copyPng.textContent = 'Copy failed';
  }
  setTimeout(() => (copyPng.textContent = original), 1500);
});

function scheduleRegenerate() {
  clearTimeout(genTimer);
  genTimer = setTimeout(regenerate, 120);
}

genText.addEventListener('input', scheduleRegenerate);
genEc.addEventListener('change', regenerate);
genMicro.addEventListener('change', () => {
  syncEcOptions();
  regenerate();
});

syncEcOptions();
regenerate();
