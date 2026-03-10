// Web Worker — owns the SSE connection, processes log entries,
// maintains filtered state, and serves batches to the main thread.

import { ansiToHtml, stripAnsi } from "./lib/ansi.js";
import { linkifyHtml } from "./lib/linkify.js";
import { tokenifyHtml } from "./lib/tokens.js";
import { detectLevel } from "./lib/levels.js";

// ── State ──────────────────────────────────────────────────────────────

const allLogs = [];
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

// ── Entry processing ───────────────────────────────────────────────────

function processEntry(raw) {
  const html = tokenifyHtml(linkifyHtml(ansiToHtml(raw.line)));
  const plain = stripAnsi(raw.line);
  const level = detectLevel(plain);
  return {
    index: raw.index,
    process: raw.process,
    color: raw.color,
    html,
    line: plain,
    level,
    system: raw.sys,
    timestamp: raw.ts,
  };
}

// ── Filtering ──────────────────────────────────────────────────────────

function buildRegex() {
  if (!filter.query) { compiledRegex = null; return; }
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

function matchesFilter(entry) {
  const hidden = filter.hiddenProcesses;
  if (hidden.length > 0 && hidden.includes(entry.process)) return false;
  const hLevels = filter.hiddenLevels;
  if (hLevels.length > 0) {
    const lvl = entry.level || "none";
    if (hLevels.includes(lvl)) return false;
  }
  // Token filter — line must contain ALL active tokens (AND logic)
  const tokens = filter.activeTokens;
  if (tokens.length > 0) {
    const lower = entry.line.toLowerCase();
    for (let t = 0; t < tokens.length; t++) {
      if (!lower.includes(tokens[t])) return false;
    }
  }
  if (!filter.query || !compiledRegex) return true;
  compiledRegex.lastIndex = 0;
  if (compiledRegex.test(entry.line)) return true;
  compiledRegex.lastIndex = 0;
  if (compiledRegex.test(entry.process)) return true;
  return false;
}

function rebuildFilter() {
  buildRegex();
  filteredIndices = [];
  for (let i = 0; i < allLogs.length; i++) {
    if (matchesFilter(allLogs[i])) filteredIndices.push(i);
  }
}

function applyHighlights(entry) {
  if (!compiledRegex) return entry;
  const html = entry.html.replace(/(<[^>]+>)|([^<]+)/g, (m, tag, text) => {
    if (tag) return tag;
    compiledRegex.lastIndex = 0;
    return text.replace(compiledRegex, '<span class="search-highlight">$&</span>');
  });
  return { ...entry, html };
}

// ── Update notifications ───────────────────────────────────────────────

function sendUpdate() {
  updateTimer = null;
  self.postMessage({
    type: "update",
    total: allLogs.length,
    filtered: filteredIndices.length,
    processes: Array.from(knownProcesses.entries()),
  });
}

function scheduleUpdate() {
  if (updateTimer !== null) return;
  updateTimer = setTimeout(sendUpdate, 50);
}

// ── SSE connection ─────────────────────────────────────────────────────

function connect() {
  if (eventSource) eventSource.close();
  allLogs.length = 0;
  filteredIndices.length = 0;
  knownProcesses.clear();

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
      raw.index = parseInt(e.lastEventId, 10);

      const entry = processEntry(raw);
      allLogs.push(entry);

      if (!knownProcesses.has(entry.process)) {
        knownProcesses.set(entry.process, entry.color);
      }

      if (matchesFilter(entry)) {
        filteredIndices.push(allLogs.length - 1);
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

self.onmessage = (e) => {
  const msg = e.data;

  switch (msg.type) {
    case "getBatch": {
      const entries = [];
      const end = Math.min(msg.start + msg.count, filteredIndices.length);
      for (let i = msg.start; i < end; i++) {
        entries.push(applyHighlights(allLogs[filteredIndices[i]]));
      }
      self.postMessage({ type: "batch", id: msg.id, entries });
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
      rebuildFilter();
      sendUpdate();
      break;
    }

    case "getAll": {
      const entries = allLogs.map((e) => ({
        timestamp: e.timestamp,
        process: e.process,
        line: e.line,
      }));
      self.postMessage({ type: "allLogs", id: msg.id, entries });
      break;
    }

    case "getFiltered": {
      const entries = filteredIndices.map((i) => {
        const e = allLogs[i];
        return { timestamp: e.timestamp, process: e.process, line: e.line };
      });
      self.postMessage({ type: "filteredLogs", id: msg.id, entries });
      break;
    }
  }
};

connect();
