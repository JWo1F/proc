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
import { initContextMenu } from "./components/context-menu.js";
import { $ } from "./lib/dom.js";

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

dbWorker.onmessage = (e) => emit(e.data.type, e.data);
ingestWorker.onmessage = (e) => emit(e.data.type, e.data);

const channel = new MessageChannel();
dbWorker.postMessage({ type: "initPort", port: channel.port2 }, [channel.port2]);

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

// ── Init event — set project name and page title ──────────────────────

on("init", (msg) => {
  ui.projectName = msg.name;
  document.title = `Procfile: ${msg.name}`;
  $("app-title").textContent = msg.name;
  $("app-addr").textContent = location.host;
});

// ── Connection status badge ──────────────────────────────────────────

on("connected", (msg) => {
  const badge = $("status-badge");
  if (msg.value) {
    badge.className =
      "flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-400";
    badge.innerHTML =
      '<span class="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse"></span><span>connected</span>';
  } else {
    badge.className =
      "flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400";
    badge.innerHTML =
      '<span class="w-1.5 h-1.5 rounded-full bg-red-500"></span><span>disconnected</span>';
  }
});

// ── Ingest stats ─────────────────────────────────────────────────────

on("ingestStats", (msg) => {
  const statRecv = $("stat-recv");
  const statQueue = $("stat-queue");
  statRecv.textContent = `recv: ${msg.received.toLocaleString()}`;
  statRecv.classList.remove("hidden");
  if (msg.queued > 0) {
    statQueue.textContent = `queue: ${msg.queued.toLocaleString()}`;
    statQueue.classList.remove("hidden");
  } else {
    statQueue.classList.add("hidden");
  }
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
initContextMenu();
initHistory();
