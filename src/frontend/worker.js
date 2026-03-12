// Web Worker — owns the SSE connection, stores log entries in IndexedDB,
// maintains compact in-memory metadata for filtering, and serves
// rendered batches to the main thread on demand.

import { ansiToHtml, stripAnsi } from "./lib/ansi.js";
import { linkifyHtml } from "./lib/linkify.js";
import { tokenifyHtml } from "./lib/tokens.js";
import { detectLevel } from "./lib/levels.js";
import { openDB, clearStore, putBatch, getByIndices, iterateAll } from "./lib/idb.js";

// ── State ──────────────────────────────────────────────────────────────

// Compact in-memory metadata — only what's needed for non-text filtering
// and volume computation.  Full entry data lives in IndexedDB.
const entryMeta = []; // { process, level, ts, index }
let filteredIndices = [];
const knownProcesses = new Map(); // name → color

let filter = {
  query: "",
  caseSensitive: false,
  wholeWord: false,
  regex: false,
  hiddenProcesses: [],
  hiddenLevels: [],
  activeTokens: [],
};
let compiledRegex = null;

let eventSource = null;
let updateTimer = null;
let filterVersion = 0;

// ── IDB write buffer ──────────────────────────────────────────────────
// Entries accumulate in writeBuf and flush to IDB periodically.
// recentCache keeps the last N entries in memory so getBatch can serve
// the visible tail without waiting on IDB during heavy ingestion.

let writeBuf = [];
let writeTimer = null;

const RECENT_CAP = 4000;
const recentCache = new Map(); // index → stored entry

function addRecent(entry) {
  recentCache.set(entry.index, entry);
  if (recentCache.size > RECENT_CAP * 2) {
    const minKeep = entry.index - RECENT_CAP;
    for (const k of recentCache.keys()) {
      if (k < minKeep) recentCache.delete(k);
      else break;
    }
  }
}

function flushWrites() {
  writeTimer = null;
  if (writeBuf.length === 0) return;
  const batch = writeBuf;
  writeBuf = [];
  putBatch(batch); // fire and forget
}

function scheduleWrite(entry) {
  addRecent(entry);
  writeBuf.push(entry);
  if (writeBuf.length >= 2000) {
    if (writeTimer) { clearTimeout(writeTimer); writeTimer = null; }
    flushWrites();
  } else if (!writeTimer) {
    writeTimer = setTimeout(flushWrites, 200);
  }
}

async function flushPending() {
  if (writeTimer) { clearTimeout(writeTimer); writeTimer = null; }
  if (writeBuf.length === 0) return;
  const batch = writeBuf;
  writeBuf = [];
  await putBatch(batch);
}

// ── Entry helpers ─────────────────────────────────────────────────────

const pad2 = (n) => (n < 10 ? "0" : "") + n;

function formatTimestamp(unixSeconds) {
  const d = new Date(unixSeconds * 1000);
  return (
    pad2(d.getHours()) + ":" + pad2(d.getMinutes()) + ":" + pad2(d.getSeconds())
  );
}

// Convert a stored IDB entry into the rendered form sent to the main thread.
function renderEntry(stored) {
  const html = tokenifyHtml(linkifyHtml(ansiToHtml(stored.raw)));
  return {
    index: stored.index,
    process: stored.process,
    color: stored.color,
    html,
    line: stored.plain,
    level: stored.level,
    system: stored.system,
    timestamp: formatTimestamp(stored.ts),
    ts: stored.ts,
  };
}

// Retrieve stored entries by index, checking the recent cache first.
async function getStored(indices) {
  const results = new Array(indices.length);
  const idbLookups = [];

  for (let i = 0; i < indices.length; i++) {
    const idx = indices[i];
    if (recentCache.has(idx)) {
      results[i] = recentCache.get(idx);
    } else {
      idbLookups.push({ pos: i, index: idx });
    }
  }

  if (idbLookups.length > 0) {
    const fromIdb = await getByIndices(idbLookups.map((l) => l.index));
    for (let i = 0; i < idbLookups.length; i++) {
      results[idbLookups[i].pos] = fromIdb[i];
    }
  }

  return results;
}

// ── Filtering ──────────────────────────────────────────────────────────

function buildRegex() {
  if (!filter.query) {
    compiledRegex = null;
    return;
  }
  try {
    const flags = filter.caseSensitive ? "g" : "gi";
    let pattern = filter.regex
      ? filter.query
      : filter.query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    if (filter.wholeWord) pattern = `\\b${pattern}\\b`;
    compiledRegex = new RegExp(pattern, flags);
  } catch {
    compiledRegex = null;
  }
}

// Fast path: only checks process and level (no text access needed).
function matchesMeta(meta) {
  const hidden = filter.hiddenProcesses;
  if (hidden.length > 0 && hidden.includes(meta.process)) return false;
  const hLevels = filter.hiddenLevels;
  if (hLevels.length > 0) {
    const lvl = meta.level || "none";
    if (hLevels.includes(lvl)) return false;
  }
  return true;
}

// Full filter including text — caller provides plain text.
function matchesFull(meta, plain) {
  if (!matchesMeta(meta)) return false;
  const tokens = filter.activeTokens;
  if (tokens.length > 0) {
    const lower = plain.toLowerCase();
    for (let t = 0; t < tokens.length; t++) {
      if (!lower.includes(tokens[t])) return false;
    }
  }
  if (!filter.query || !compiledRegex) return true;
  compiledRegex.lastIndex = 0;
  if (compiledRegex.test(plain)) return true;
  compiledRegex.lastIndex = 0;
  if (compiledRegex.test(meta.process)) return true;
  return false;
}

let rebuildVersion = 0;

async function rebuildFilter() {
  buildRegex();
  filteredIndices = [];

  const needsText =
    (filter.query && compiledRegex) || filter.activeTokens.length > 0;

  if (needsText) {
    // Flush pending writes so IDB has everything.
    await flushPending();

    const myVersion = ++rebuildVersion;

    await iterateAll((entry) => {
      // Abort if a newer rebuild was requested.
      if (rebuildVersion !== myVersion) return;
      const meta = { process: entry.process, level: entry.level };
      if (!matchesMeta(meta)) return;
      const tokens = filter.activeTokens;
      if (tokens.length > 0) {
        const lower = entry.plain.toLowerCase();
        for (let t = 0; t < tokens.length; t++) {
          if (!lower.includes(tokens[t])) return;
        }
      }
      if (compiledRegex) {
        compiledRegex.lastIndex = 0;
        if (!compiledRegex.test(entry.plain)) {
          compiledRegex.lastIndex = 0;
          if (!compiledRegex.test(entry.process)) return;
        }
      }
      filteredIndices.push(entry.index);
    });
  } else {
    // Process/level only — use compact in-memory metadata.
    for (let i = 0; i < entryMeta.length; i++) {
      if (matchesMeta(entryMeta[i])) filteredIndices.push(i);
    }
  }
}

function applyHighlights(entry) {
  if (!compiledRegex) return entry;
  const html = entry.html.replace(/(<[^>]+>)|([^<]+)/g, (m, tag, text) => {
    if (tag) return tag;
    compiledRegex.lastIndex = 0;
    return text.replace(
      compiledRegex,
      '<span class="search-highlight">$&</span>',
    );
  });
  return { ...entry, html };
}

// ── Volume (incremental) ──────────────────────────────────────────────
// Instead of recomputing from scratch on every update, we track a cached
// volume and only recompute when the time range or entry count changes
// significantly.

let volumeCache = null;
let volumeComputedAt = 0; // entryMeta.length when last computed

function computeVolume() {
  const len = entryMeta.length;
  if (len === 0) return null;

  // Reuse cached result if fewer than 5% new entries since last compute,
  // and at least 1000 entries (avoid stale volume on small datasets).
  if (volumeCache && len > 1000 && len - volumeComputedAt < len * 0.05) {
    return volumeCache;
  }

  const first = entryMeta[0].ts;
  const last = entryMeta[len - 1].ts;
  const range = last - first;

  const BUCKET_TARGET = 60;
  const bucketCount = range < 1 ? 1 : Math.min(BUCKET_TARGET, Math.ceil(range));
  const interval = range / bucketCount || 1;

  const buckets = [];
  for (let i = 0; i < bucketCount; i++) {
    buckets.push({ debug: 0, info: 0, warn: 0, error: 0, fatal: 0, none: 0 });
  }

  for (let i = 0; i < len; i++) {
    const e = entryMeta[i];
    const idx = Math.min(
      Math.floor((e.ts - first) / interval),
      bucketCount - 1,
    );
    buckets[idx][e.level || "none"]++;
  }

  volumeCache = { buckets, start: first, interval };
  volumeComputedAt = len;
  return volumeCache;
}

// ── Update notifications ───────────────────────────────────────────────

function sendUpdate() {
  updateTimer = null;
  self.postMessage({
    type: "update",
    total: entryMeta.length,
    filtered: filteredIndices.length,
    processes: Array.from(knownProcesses.entries()),
    filterVersion,
    volume: computeVolume(),
  });
}

function scheduleUpdate() {
  if (updateTimer !== null) return;
  // Adaptive throttle: longer delay during heavy ingestion to avoid
  // spending all CPU on volume recomputation and postMessage overhead.
  const delay = entryMeta.length > 10000 ? 200 : 50;
  updateTimer = setTimeout(sendUpdate, delay);
}

// ── SSE connection ─────────────────────────────────────────────────────

async function connect() {
  if (eventSource) eventSource.close();
  entryMeta.length = 0;
  filteredIndices.length = 0;
  knownProcesses.clear();
  recentCache.clear();
  writeBuf.length = 0;
  volumeCache = null;
  volumeComputedAt = 0;
  if (writeTimer) { clearTimeout(writeTimer); writeTimer = null; }

  await clearStore();

  eventSource = new EventSource("/api/sse");

  eventSource.addEventListener("init", (e) => {
    try {
      const info = JSON.parse(e.data);
      self.postMessage({ type: "init", name: info.name || "procfile" });
    } catch {}
  });

  eventSource.addEventListener("message", (e) => {
    try {
      const raw = JSON.parse(e.data);
      const index = parseInt(e.lastEventId, 10);
      const plain = stripAnsi(raw.line);
      const level = detectLevel(plain);

      // Compact in-memory metadata for filtering & volume.
      const meta = { process: raw.process, level, ts: raw.ts, index };
      entryMeta.push(meta);

      // Full entry for IndexedDB (deferred HTML processing).
      scheduleWrite({
        index,
        process: raw.process,
        color: raw.color,
        raw: raw.line,
        plain,
        level,
        system: raw.sys,
        ts: raw.ts,
      });

      if (!knownProcesses.has(raw.process)) {
        knownProcesses.set(raw.process, raw.color);
      }

      // Incremental filter check — we have the plain text right here.
      if (matchesFull(meta, plain)) {
        filteredIndices.push(entryMeta.length - 1);
      }

      scheduleUpdate();
    } catch (err) {
      console.error("Failed to parse SSE event:", err);
    }
  });

  eventSource.addEventListener("open", () => {
    self.postMessage({ type: "connected", value: true });
  });

  eventSource.addEventListener("error", () => {
    self.postMessage({ type: "connected", value: false });
    eventSource.close();
  });
}

// ── Message handler ────────────────────────────────────────────────────

self.onmessage = async (e) => {
  const msg = e.data;

  switch (msg.type) {
    case "getBatch": {
      const end = Math.min(msg.start + msg.count, filteredIndices.length);
      const indices = [];
      for (let i = msg.start; i < end; i++) {
        indices.push(filteredIndices[i]);
      }

      const stored = await getStored(indices);
      const entries = [];
      for (const s of stored) {
        if (s) entries.push(applyHighlights(renderEntry(s)));
      }

      self.postMessage({
        type: "batch",
        id: msg.id,
        start: msg.start,
        entries,
      });
      break;
    }

    case "setFilter": {
      filter = {
        query: msg.query || "",
        caseSensitive: !!msg.caseSensitive,
        wholeWord: !!msg.wholeWord,
        regex: !!msg.regex,
        hiddenProcesses: msg.hiddenProcesses || [],
        hiddenLevels: msg.hiddenLevels || [],
        activeTokens: (msg.activeTokens || []).map((t) => t.toLowerCase()),
      };
      filterVersion++;
      volumeCache = null; // force recompute on next update
      await rebuildFilter();
      sendUpdate();
      break;
    }

    case "getAll": {
      await flushPending();
      const entries = [];
      await iterateAll((e) => {
        entries.push({
          timestamp: formatTimestamp(e.ts),
          process: e.process,
          line: e.plain,
        });
      });
      self.postMessage({ type: "allLogs", id: msg.id, entries });
      break;
    }

    case "getFiltered": {
      await flushPending();
      const indexSet = new Set(filteredIndices);
      const entries = [];
      await iterateAll((e) => {
        if (indexSet.has(e.index)) {
          entries.push({
            timestamp: formatTimestamp(e.ts),
            process: e.process,
            line: e.plain,
          });
        }
      });
      self.postMessage({ type: "filteredLogs", id: msg.id, entries });
      break;
    }
  }
};

connect();
