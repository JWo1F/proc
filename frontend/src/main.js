import "./style.css";

import { initTheme } from "./components/theme.js";
import { initAutoScroll } from "./components/auto-scroll.js";
import { initSearch } from "./components/search.js";
import { initProcessFilter } from "./components/process-filter.js";
import { initLevelFilter } from "./components/level-filter.js";
import { initDownloads } from "./components/downloads.js";
import { initVirtualScroll } from "./components/virtual-scroll.js";
import { initTokenFilter } from "./components/token-filter.js";

// ── Shared UI state ────────────────────────────────────────────────────

export const ui = {
  worker: null,
  autoScroll: true,
  projectName: "procfile",
  totalLogs: 0,
  filteredLogs: 0,
  processes: new Map(),       // name → color
  hiddenProcesses: new Set(),
  hiddenLevels: new Set(),
  searchQuery: "",
  searchCaseSensitive: false,
  searchWholeWord: false,
  searchRegex: false,
  activeTokens: new Set(),   // token values for cross-process correlation
};

// ── Worker setup ───────────────────────────────────────────────────────

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
ui.worker = worker;

// Dispatch worker messages to registered handlers
const handlers = new Map();

export function onWorkerMessage(type, fn) {
  if (!handlers.has(type)) handlers.set(type, []);
  handlers.get(type).push(fn);
}

worker.onmessage = (e) => {
  const fns = handlers.get(e.data.type);
  if (fns) fns.forEach((fn) => fn(e.data));
};

// ── Sync shared state from worker before any component handlers run ───

onWorkerMessage("update", (msg) => {
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
initVirtualScroll();
