// SSE connection management — connects to the backend, parses events,
// and batches them for efficient rendering.

import { state } from "./state.js";
import { $ } from "./dom.js";
import { statusBadge, logViewport, emptyState } from "./dom.js";
import { matchesFilter } from "./filters.js";
import { clearBlocks, renderVisible, scrollToBottom } from "../components/virtual-scroll.js";
import { registerProcess } from "../components/process-filter.js";

const RENDER_DEBOUNCE_MS = 50;
const ROW_HEIGHT = 24;
const BLOCK_SIZE = 100;

let renderTimer = null;

function setStatus(connected) {
  if (connected) {
    statusBadge.className =
      "flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-400";
    statusBadge.innerHTML =
      '<span class="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse"></span><span>connected</span>';
  } else {
    statusBadge.className =
      "flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400";
    statusBadge.innerHTML =
      '<span class="w-1.5 h-1.5 rounded-full bg-red-500"></span><span>disconnected</span>';
  }
}

function parseLogEvent(e) {
  const d = JSON.parse(e.data);
  return {
    index: parseInt(e.lastEventId, 10),
    process: d.process,
    color: d.color,
    timestamp: d.ts,
    system: d.sys,
    line: d.line,
    html: d.html,
  };
}

function enqueueEntry(entry) {
  if (entry.index < state.allLogs.length) return; // duplicate
  state.allLogs.push(entry);
  registerProcess(entry.process, entry.color);
  state.pendingEntries.push(entry);
  scheduleFlush();
}

function scheduleFlush() {
  if (renderTimer !== null) return;
  renderTimer = setTimeout(flushPending, RENDER_DEBOUNCE_MS);
}

function flushPending() {
  renderTimer = null;
  if (state.pendingEntries.length === 0) return;

  const batch = state.pendingEntries;
  state.pendingEntries = [];

  const prevTotal = state.filteredLogs.length;
  const startAllIdx = state.allLogs.length - batch.length;
  for (let i = 0; i < batch.length; i++) {
    if (matchesFilter(batch[i])) {
      state.filteredLogs.push(startAllIdx + i);
    }
  }

  emptyState.classList.toggle("hidden", state.allLogs.length > 0);

  // Invalidate the last block — it may now have more rows
  if (prevTotal > 0) {
    const lastBlock = Math.floor((prevTotal - 1) / BLOCK_SIZE);
    const el = state.renderedBlocks.get(lastBlock);
    if (el) {
      el.remove();
      state.renderedBlocks.delete(lastBlock);
    }
  }

  logViewport.style.height = state.filteredLogs.length * ROW_HEIGHT + "px";

  if (state.autoScroll) scrollToBottom();
  renderVisible();
}

export function connectSSE() {
  if (state.eventSource) state.eventSource.close();
  state.allLogs = [];
  state.filteredLogs = [];
  state.pendingEntries = [];
  clearBlocks();
  logViewport.style.height = "0px";
  state.eventSource = new EventSource("/api/sse");

  state.eventSource.addEventListener("init", (e) => {
    try {
      const info = JSON.parse(e.data);
      state.projectName = info.name || "procfile";
      document.title = `Procfile: ${state.projectName}`;
      $("app-title").textContent = state.projectName;
      $("app-addr").textContent = location.host;
    } catch (_) {}
  });

  state.eventSource.addEventListener("log", (e) => {
    try {
      enqueueEntry(parseLogEvent(e));
    } catch (err) {
      console.error("Failed to parse SSE event:", err);
    }
  });

  state.eventSource.addEventListener("open", () => setStatus(true));
  state.eventSource.addEventListener("error", () => {
    setStatus(false);
    state.eventSource.close();
  });
}
