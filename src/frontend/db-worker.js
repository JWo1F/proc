// Database worker — owns the SQLite database, filter state, entry rendering,
// and serves all queries. Receives parsed log batches from the ingest worker
// via MessagePort, and commands from the main thread via self.onmessage.

import {
  initDB, clearAll, putBatch as dbPutBatch, getByIndices, getAllSorted,
  queryFilteredIndices, getFilteredEntries,
  getEntryCount, getProcesses, computeVolume, getDBSize,
  syncFTSChunk, getFTSCount,
} from "./lib/sqlite.js";
import { ansiToHtml } from "./lib/ansi.js";
import { linkifyHtml } from "./lib/linkify.js";
import { tokenifyHtml } from "./lib/tokens.js";

const FTS_CHUNK = 500;

// ── Insert queue ────────────────────────────────────────────────────

const insertQueue = [];

// ── Filter state ────────────────────────────────────────────────────

let filteredIndices = [];

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
let filterVersion = 0;
let rebuildVersion = 0;

// ── Entry rendering ─────────────────────────────────────────────────

const pad2 = (n) => (n < 10 ? "0" : "") + n;
const pad3 = (n) => (n < 10 ? "00" : n < 100 ? "0" : "") + n;

function formatTimestamp(unixSeconds, ms) {
  const d = new Date(unixSeconds * 1000);
  const base =
    pad2(d.getHours()) + ":" + pad2(d.getMinutes()) + ":" + pad2(d.getSeconds());
  if (ms) return base + "." + pad3(d.getMilliseconds());
  return base;
}

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
    timestamp: formatTimestamp(stored.ts, stored.hasMs),
    ts: stored.ts,
  };
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

// ── Filtering ───────────────────────────────────────────────────────

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

function matchesEntry(process, plain) {
  const hidden = filter.hiddenProcesses;
  if (hidden.length > 0 && hidden.includes(process)) return false;
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
  if (compiledRegex.test(process)) return true;
  return false;
}

function matchesText(entry) {
  const tokens = filter.activeTokens;
  if (tokens.length > 0) {
    const lower = entry.plain.toLowerCase();
    for (let t = 0; t < tokens.length; t++) {
      if (!lower.includes(tokens[t])) return false;
    }
  }
  if (!compiledRegex) return true;
  compiledRegex.lastIndex = 0;
  if (compiledRegex.test(entry.plain)) return true;
  compiledRegex.lastIndex = 0;
  if (compiledRegex.test(entry.process)) return true;
  return false;
}

function rebuildFilter() {
  buildRegex();
  const myVersion = ++rebuildVersion;

  let candidates = queryFilteredIndices({
    hiddenProcesses: filter.hiddenProcesses,
    hiddenLevels: filter.hiddenLevels,
    query: (!filter.regex && filter.query) || null,
    activeTokens: filter.activeTokens,
  });

  if (rebuildVersion !== myVersion) return;

  // Post-filter with JS regex (SQL can't do JS regex)
  if (filter.regex && compiledRegex && candidates.length > 0) {
    const stored = getByIndices(candidates);
    if (rebuildVersion !== myVersion) return;
    const verified = [];
    for (const entry of stored) {
      if (entry && matchesText(entry)) verified.push(entry.index);
    }
    candidates = verified;
  }

  if (rebuildVersion !== myVersion) return;
  filteredIndices = candidates;
}

// ── Update notifications ────────────────────────────────────────────

let updateTimer = null;
let lastFullUpdate = 0;
let cachedProcesses = [];
let cachedVolume = null;
let cachedDbSize = 0;

function sendUpdate() {
  updateTimer = null;
  const total = getEntryCount();
  const indexed = getFTSCount();

  // Expensive aggregates: recompute at most once per second
  const now = performance.now();
  if (now - lastFullUpdate > 1000) {
    lastFullUpdate = now;
    cachedProcesses = getProcesses();
    cachedVolume = computeVolume({
      hiddenProcesses: filter.hiddenProcesses,
      hiddenLevels: filter.hiddenLevels,
      query: (!filter.regex && filter.query) || null,
      activeTokens: filter.activeTokens,
    });
    cachedDbSize = getDBSize();
  }

  self.postMessage({
    type: "update",
    total,
    filtered: filteredIndices.length,
    processes: cachedProcesses,
    filterVersion,
    volume: cachedVolume,
    dbSize: cachedDbSize,
    indexed,
  });
}

function sendFullUpdate() {
  lastFullUpdate = 0; // force expensive recompute
  if (updateTimer !== null) { clearTimeout(updateTimer); updateTimer = null; }
  sendUpdate();
}

function scheduleUpdate() {
  if (updateTimer !== null) return;
  updateTimer = setTimeout(sendUpdate, 100);
}

// ── Sorted rebuild scheduling ───────────────────────────────────────

let rebuildTimer = null;

function scheduleSortedRebuild() {
  if (rebuildTimer !== null) clearTimeout(rebuildTimer);
  rebuildTimer = setTimeout(() => {
    rebuildTimer = null;
    rebuildFilter();
    filterVersion++;
    sendFullUpdate();
  }, 500);
}

// ── Priority event loop ─────────────────────────────────────────────
// Drains insert queue first (priority), then does FTS chunks in idle time.
// Yields between each unit of work so new inserts / messages are picked up.

let loopRunning = false;

function kickLoop() {
  if (loopRunning) return;
  loopRunning = true;
  setTimeout(runLoop, 0);
}

function runLoop() {
  // 1. Drain all pending inserts first
  if (insertQueue.length > 0) {
    const batch = insertQueue.shift();
    dbPutBatch(batch);

    // Inline filter matching for new entries
    for (const entry of batch) {
      if (matchesEntry(entry.process, entry.plain)) {
        filteredIndices.push(entry.index);
      }
    }

    scheduleSortedRebuild();
    scheduleUpdate();

    // Yield, then continue loop (more inserts or FTS)
    setTimeout(runLoop, 0);
    return;
  }

  // 2. No inserts pending — do one FTS chunk
  const synced = syncFTSChunk(FTS_CHUNK);
  if (synced > 0) {
    scheduleUpdate();
    // Yield, then check for new inserts before next FTS chunk
    setTimeout(runLoop, 0);
    return;
  }

  // 3. Nothing to do — stop loop
  loopRunning = false;
}

// ── Ingest worker port (receives putBatch) ──────────────────────────

let ingestPort = null;

function handleIngestMessage(e) {
  const msg = e.data;
  if (msg.type === "putBatch") {
    insertQueue.push(msg.entries);
    kickLoop();
  }
}

// ── Main thread message handler ─────────────────────────────────────

self.onmessage = async (e) => {
  const msg = e.data;
  switch (msg.type) {
    case "init":
      await initDB();
      self.postMessage({ type: "ready" });
      break;

    case "clear":
      clearAll();
      insertQueue.length = 0;
      filteredIndices = [];
      filterVersion++;
      self.postMessage({ type: "ready" });
      break;

    case "initPort":
      ingestPort = msg.port;
      ingestPort.onmessage = handleIngestMessage;
      break;

    case "getBatch": {
      const end = Math.min(msg.start + msg.count, filteredIndices.length);
      const indices = [];
      for (let i = msg.start; i < end; i++) {
        indices.push(filteredIndices[i]);
      }

      const stored = getByIndices(indices);
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
      if (rebuildTimer) { clearTimeout(rebuildTimer); rebuildTimer = null; }
      rebuildFilter();
      sendFullUpdate();
      break;
    }

    case "findPosition": {
      const pos = filteredIndices.indexOf(msg.rawIndex);
      self.postMessage({ type: "position", rawIndex: msg.rawIndex, position: pos });
      break;
    }

    case "getAll": {
      const all = getAllSorted();
      const entries = all.map((e) => ({
        timestamp: formatTimestamp(e.ts, e.hasMs),
        process: e.process,
        line: e.plain,
      }));
      self.postMessage({ type: "allLogs", id: msg.id, entries });
      break;
    }

    case "getFiltered": {
      const entries = getFilteredEntries({
        hiddenProcesses: filter.hiddenProcesses,
        hiddenLevels: filter.hiddenLevels,
        query: (!filter.regex && filter.query) || null,
        activeTokens: filter.activeTokens,
      }).map((e) => ({
        timestamp: formatTimestamp(e.ts, e.hasMs),
        process: e.process,
        line: e.plain,
      }));
      self.postMessage({ type: "filteredLogs", id: msg.id, entries });
      break;
    }
  }
};
