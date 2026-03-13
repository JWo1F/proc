import "./style.css";

import { initTheme } from "./components/theme.js";
import { initAutoScroll } from "./components/auto-scroll.js";
import { initSearch } from "./components/search.js";
import { initProcessFilter } from "./components/process-filter.js";
import { initLevelFilter } from "./components/level-filter.js";
import { initDownloads } from "./components/downloads.js";
import { initVirtualScroll } from "./components/virtual-scroll.js";
import { initTokenFilter } from "./components/token-filter.js";
import { initHistory } from "./components/history.js";
import { initLogsVolume } from "./components/logs-volume.js";

// ── Shared UI state ────────────────────────────────────────────────────

export const ui = {
  dbWorker: null,
  autoScroll: true,
  projectName: "procfile",
  totalLogs: 0,
  filteredLogs: 0,
  processes: new Map(), // name -> color
  hiddenProcesses: new Set(),
  hiddenLevels: new Set(),
  searchQuery: "",
  searchCaseSensitive: false,
  searchWholeWord: false,
  searchRegex: false,
  activeTokens: new Set(), // token values for cross-process correlation
};

// ── Event bus ──────────────────────────────────────────────────────────

const handlers = new Map();

export function on(type, fn) {
  if (!handlers.has(type)) handlers.set(type, []);
  handlers.get(type).push(fn);
}

export function off(type, fn) {
  const fns = handlers.get(type);
  if (!fns) return;
  const idx = fns.indexOf(fn);
  if (idx >= 0) fns.splice(idx, 1);
}

function emit(type, data) {
  const fns = handlers.get(type);
  if (fns) fns.forEach((fn) => fn(data));
}

// ── Worker setup ───────────────────────────────────────────────────────

const dbWorker = new Worker(new URL("./db-worker.js", import.meta.url), {
  type: "module",
});
ui.dbWorker = dbWorker;

const ingestWorker = new Worker(new URL("./ingest-worker.js", import.meta.url), {
  type: "module",
});

// Wire both workers into the event bus
dbWorker.onmessage = (e) => emit(e.data.type, e.data);
ingestWorker.onmessage = (e) => emit(e.data.type, e.data);

// Connect workers: MessageChannel for ingest -> db communication
const channel = new MessageChannel();
dbWorker.postMessage({ type: "initPort", port: channel.port2 }, [channel.port2]);

// Initialize DB, then start ingest (one-shot: port can only be transferred once)
let ingestStarted = false;
on("ready", () => {
  if (ingestStarted) return;
  ingestStarted = true;
  ingestWorker.postMessage(
    { type: "connect", dbPort: channel.port1 },
    [channel.port1],
  );
});
dbWorker.postMessage({ type: "init" });

// ── Sync shared state from worker before any component handlers run ───

on("update", (msg) => {
  ui.totalLogs = msg.total;
  ui.filteredLogs = msg.filtered;
  ui.processes = new Map(msg.processes);
});

// ── Init ───────────────────────────────────────────────────────────────

initTheme();
initAutoScroll();
initSearch();
initProcessFilter();
initLevelFilter();
initDownloads();
initTokenFilter();
initLogsVolume();
initVirtualScroll();
initHistory();
