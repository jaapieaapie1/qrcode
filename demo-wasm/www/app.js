// Browser glue for the qr-demo WebAssembly module.
//
// Scanning: a frame is drawn to an offscreen canvas, converted to an 8-bit
// luma plane here (the wasm side takes raw luma so the demo needs no web-sys
// binding), then handed to `scan`, which returns JSON.
// Generating: `generate` returns an SVG document string, or a message starting
// with "ERROR:".

import init, { scan, generate } from './pkg/qr_demo.js';

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

// Longest edge the detector sees. Full-resolution camera frames buy no
// accuracy at these module sizes and cost the whole frame budget.
const MAX_EDGE = 720;
// ~10 scans per second; the rAF loop simply skips frames in between.
const SCAN_INTERVAL_MS = 100;
// How many distinct payloads to keep in the decoded list.
const MAX_RESULTS = 8;

const work = document.createElement('canvas');
const workCtx = work.getContext('2d', { willReadFrequently: true });
const overlayCtx = overlay.getContext('2d');

let stream = null;
let rafId = 0;
let lastScan = 0;
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

/** Draws `source` into the work canvas at a bounded size and returns its luma plane. */
function toLuma(source, sw, sh) {
  const factor = Math.min(1, MAX_EDGE / Math.max(sw, sh));
  const w = Math.max(1, Math.round(sw * factor));
  const h = Math.max(1, Math.round(sh * factor));
  if (work.width !== w || work.height !== h) {
    work.width = w;
    work.height = h;
  }
  workCtx.drawImage(source, 0, 0, w, h);
  const { data } = workCtx.getImageData(0, 0, w, h);
  const luma = new Uint8Array(w * h);
  for (let i = 0, p = 0; i < luma.length; i++, p += 4) {
    // Integer approximation of 0.299R + 0.587G + 0.114B.
    luma[i] = (77 * data[p] + 150 * data[p + 1] + 29 * data[p + 2]) >> 8;
  }
  return { luma, w, h };
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

function drawOverlay(detections, sw, sh) {
  const map = overlayMapper(sw, sh);
  overlayCtx.clearRect(0, 0, overlay.width, overlay.height);
  overlayCtx.lineWidth = 3;
  overlayCtx.lineJoin = 'round';
  overlayCtx.font = '13px system-ui, sans-serif';

  for (const d of detections) {
    const decoded = d.error === null;
    overlayCtx.strokeStyle = decoded ? '#39d98a' : '#ffb020';
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
    const label = d.kind === 'micro'
      ? `Micro${d.version ? ' M' + d.version : ''}`
      : `QR${d.version ? ' v' + d.version : ''}`;
    overlayCtx.fillText(decoded ? label : `${label} — unreadable`, lx, Math.max(12, ly - 6));
  }
}

/**
 * Folds decoded payloads into the distinct-results list. A symbol held in
 * front of the camera decodes ten times a second; only its first sighting and
 * a once-a-second timestamp refresh are worth a re-render.
 */
function recordResults(detections) {
  const now = Date.now();
  let changed = false;
  for (const d of detections) {
    if (d.error !== null) continue;
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
    tag.textContent = e.kind === 'micro' ? `Micro M${e.version ?? '?'}` : `QR v${e.version ?? '?'}`;
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

async function runScan(source, sw, sh) {
  if (!(await ready)) return;
  const { luma, w, h } = toLuma(source, sw, sh);
  let detections;
  try {
    detections = JSON.parse(scan(luma, w, h));
  } catch (err) {
    setStatus(`Scan failed: ${err}`, true);
    return;
  }
  drawOverlay(detections, w, h);
  recordResults(detections);

  const unreadable = detections.filter((d) => d.error !== null).length;
  const decoded = detections.length - unreadable;
  if (detections.length === 0) setStatus('No symbol in frame.');
  else if (unreadable === 0) setStatus(`${decoded} symbol${decoded === 1 ? '' : 's'} decoded.`);
  else setStatus(`${decoded} decoded, ${unreadable} found but unreadable — move closer or steady the camera.`);
}

function loop(now) {
  rafId = requestAnimationFrame(loop);
  if (now - lastScan < SCAN_INTERVAL_MS) return;
  lastScan = now;
  if (video.readyState < 2 || video.videoWidth === 0) return;
  runScan(video, video.videoWidth, video.videoHeight);
}

async function startCamera() {
  if (!navigator.mediaDevices?.getUserMedia) {
    setStatus('This browser exposes no camera API. Use the image file input instead.', true);
    return;
  }
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      video: { facingMode: { ideal: 'environment' }, width: { ideal: 1280 } },
      audio: false,
    });
  } catch (err) {
    setStatus(`Camera unavailable: ${err.name === 'NotAllowedError' ? 'permission denied' : err.message}. The page must be served from localhost or over HTTPS.`, true);
    return;
  }
  preview.hidden = true;
  video.hidden = false;
  stageHint.hidden = true;
  video.srcObject = stream;
  await video.play();
  startCam.disabled = true;
  stopCam.disabled = false;
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
  if (preview.hidden) stageHint.hidden = false;
  setStatus('Camera stopped.');
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
    requestAnimationFrame(() => runScan(preview, preview.width, preview.height));
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
