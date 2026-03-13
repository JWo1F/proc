// Virtual scrolling powered by @tanstack/virtual-core.
// Requests visible entries from the worker on demand.

import {
  Virtualizer,
  observeElementOffset,
  observeElementRect,
} from "@tanstack/virtual-core";
import { ui, on } from "../main.js";
import {
  logViewport,
  logContainer,
  emptyState,
  logCountEl,
  filterCount,
  downloadFiltered,
  statIndexed,
  statDbSize,
} from "../lib/dom.js";
import { fmtSize } from "../lib/format.js";
import { updateAutoScrollBtn, syncAutoScroll } from "./auto-scroll.js";
import { clearAllFilters } from "./search.js";

const LEVEL_LETTERS = {
  debug: "D",
  info: "I",
  warn: "W",
  error: "E",
  fatal: "F",
};
const ROW_HEIGHT = 24;
const OVERSCAN = 50;

// ── Cache ─────────────────────────────────────────────────────────────

export const entryCache = new Map(); // filteredIndex → entry
export const expandedLines = new Set(); // raw log indices of expanded lines
let lastFilterVersion = -1;
let pendingKey = null;
let pendingScrollRestore = null;

// ── DOM ───────────────────────────────────────────────────────────────

let contentEl = null;
let virtualizer = null;
let baseOpts = null;
let rendering = false;
let needsRerender = false;
let renderScheduled = false;
let measureAll = false;
let lastRenderKey = "";
let selectedRawIndex = -1;
let pendingScrollToRaw = -1;

// ── Expand/collapse toggle ────────────────────────────────────────────

export function toggleExpand(rawIndex) {
  if (expandedLines.has(rawIndex)) {
    expandedLines.delete(rawIndex);
  } else {
    expandedLines.add(rawIndex);
  }
  ui.autoScroll = false;
  updateAutoScrollBtn();
  measureAll = true;
  lastRenderKey = "";
  render();
}

// ── Log element factory ───────────────────────────────────────────────

function createLogElement(entry, sortedPos) {
  const div = document.createElement("div");
  let cls =
    "log-line flex items-start gap-3 px-2 py-0.5 rounded font-mono text-[13px] leading-relaxed";
  if (entry.level) cls += ` level-${entry.level}`;
  div.className = cls;

  const idx = document.createElement("span");
  idx.className =
    "flex-none w-12 text-right text-gray-400 dark:text-gray-600 select-none text-xs leading-relaxed cursor-pointer";
  idx.textContent = "0x" + sortedPos.toString(16).toUpperCase();

  const ts = document.createElement("span");
  ts.className =
    "flex-none w-[5.5rem] text-gray-400 dark:text-gray-500 text-xs leading-relaxed cursor-pointer hover:text-blue-500 dark:hover:text-blue-400";
  ts.textContent = entry.timestamp;

  const proc = document.createElement("span");
  proc.className =
    "flex-none w-20 truncate text-xs font-medium leading-relaxed";
  proc.style.color = entry.color;
  proc.textContent = entry.process;

  const lvl = document.createElement("span");
  lvl.className =
    "flex-none w-4 text-center text-xs font-bold leading-relaxed level-badge level-badge-" +
    (entry.level || "none");
  lvl.textContent = LEVEL_LETTERS[entry.level] || "-";

  const content = document.createElement("span");
  content.className = "flex-1 min-w-0 leading-relaxed whitespace-pre";
  if (entry.system) {
    content.classList.add("font-medium");
    content.style.color = entry.color;
  }
  content.innerHTML = entry.html;

  const expanded = expandedLines.has(entry.index);
  if (expanded) {
    div.classList.add("expanded");
    content.classList.remove("whitespace-pre");
    content.classList.add("whitespace-pre-wrap", "break-all");
  }

  if (entry.index === selectedRawIndex) {
    div.classList.add("log-line-selected");
  }

  idx.addEventListener("click", (e) => {
    e.stopPropagation();
    toggleExpand(entry.index);
  });

  ts.addEventListener("click", (e) => {
    e.stopPropagation();
    if (selectedRawIndex === entry.index) {
      selectedRawIndex = -1;
      lastRenderKey = "";
      render();
      return;
    }
    selectedRawIndex = entry.index;
    ui.autoScroll = false;
    updateAutoScrollBtn();
    pendingScrollToRaw = entry.index;
    clearAllFilters();
    renderAllLogs();
  });

  div.appendChild(idx);
  div.appendChild(ts);
  div.appendChild(proc);
  div.appendChild(lvl);
  div.appendChild(content);

  return div;
}

// ── Counts ────────────────────────────────────────────────────────────

function updateCounts() {
  logCountEl.textContent = `stored: ${ui.totalLogs.toLocaleString()}`;
  const hasFilter =
    ui.searchQuery || ui.hiddenProcesses.size > 0 || ui.activeTokens.size > 0;
  if (hasFilter && ui.filteredLogs !== ui.totalLogs) {
    filterCount.textContent = `${ui.filteredLogs} / ${ui.totalLogs}`;
    filterCount.classList.remove("hidden");
    downloadFiltered.classList.remove("hidden");
    downloadFiltered.textContent = `Download filtered (${ui.filteredLogs})`;
  } else {
    filterCount.classList.add("hidden");
    downloadFiltered.classList.add("hidden");
  }
}

// ── Render ─────────────────────────────────────────────────────────────

function scheduleRender() {
  if (renderScheduled) return;
  renderScheduled = true;
  requestAnimationFrame(() => {
    renderScheduled = false;
    render();
  });
}

function render() {
  if (rendering) {
    needsRerender = true;
    return;
  }

  if (!virtualizer || ui.filteredLogs === 0) {
    if (contentEl) contentEl.innerHTML = "";
    return;
  }

  virtualizer._willUpdate();
  const items = virtualizer.getVirtualItems();
  if (items.length === 0) {
    contentEl.innerHTML = "";
    return;
  }

  let missStart = -1;
  let missEnd = -1;
  for (const item of items) {
    if (!entryCache.has(item.index)) {
      if (missStart === -1) missStart = item.index;
      missEnd = item.index + 1;
    }
  }

  if (missStart !== -1) {
    const key = `${missStart}:${missEnd}`;
    if (key !== pendingKey) {
      pendingKey = key;
      ui.dbWorker.postMessage({
        type: "getBatch",
        id: key,
        start: missStart,
        count: missEnd - missStart,
      });
    }
  }

  let renderKey = `sel:${selectedRawIndex};`;
  for (const item of items) {
    const entry = entryCache.get(item.index);
    const exp = entry && expandedLines.has(entry.index) ? 1 : 0;
    renderKey += `${item.index}:${item.start}:${entry ? 1 : 0}:${exp},`;
  }

  if (renderKey === lastRenderKey && !measureAll) {
    logViewport.style.height = virtualizer.getTotalSize() + "px";
    return;
  }
  lastRenderKey = renderKey;

  const frag = document.createDocumentFragment();
  let hasExpanded = false;
  for (const item of items) {
    const entry = entryCache.get(item.index);
    if (!entry) continue;
    const el = createLogElement(entry, item.index);
    el.setAttribute("data-index", item.index);
    el.style.cssText = `position:absolute;top:0;left:0;min-width:100%;transform:translateY(${item.start}px)`;
    frag.appendChild(el);
    if (expandedLines.has(entry.index)) hasExpanded = true;
  }

  if (frag.childNodes.length === 0) {
    lastRenderKey = "";
    return;
  }

  contentEl.innerHTML = "";
  contentEl.appendChild(frag);

  if (measureAll || hasExpanded) {
    rendering = true;
    for (const el of [...contentEl.children]) {
      virtualizer.measureElement(el);
    }
    rendering = false;
    measureAll = false;

    logViewport.style.height = virtualizer.getTotalSize() + "px";

    if (needsRerender) {
      needsRerender = false;
      render();
    }
  } else {
    logViewport.style.height = virtualizer.getTotalSize() + "px";
  }
}

// ── Public API ─────────────────────────────────────────────────────────

export function scrollToBottom() {
  if (!virtualizer || ui.filteredLogs === 0) return;
  virtualizer.scrollToIndex(ui.filteredLogs - 1, { align: "end" });
}

export function renderAllLogs() {
  entryCache.clear();
  pendingKey = null;
  if (virtualizer) {
    virtualizer.setOptions({ ...baseOpts, count: ui.filteredLogs });
    virtualizer._willUpdate();
    logViewport.style.height = virtualizer.getTotalSize() + "px";
  }
  updateCounts();
  emptyState.classList.toggle("hidden", ui.totalLogs > 0);
  if (ui.autoScroll) scrollToBottom();
  scheduleRender();
}

export function scheduleScrollRestore(scrollTop, scrollLeft) {
  pendingScrollRestore = { scrollTop, scrollLeft };
}

export function renderVisible() {
  scheduleRender();
}

export function clearBlocks() {
  entryCache.clear();
  pendingKey = null;
  lastRenderKey = "";
  if (contentEl) contentEl.innerHTML = "";
}

// ── Init ───────────────────────────────────────────────────────────────

export function initVirtualScroll() {
  contentEl = document.createElement("div");
  contentEl.style.cssText = "position:absolute;top:0;left:0;min-width:100%";
  logViewport.appendChild(contentEl);

  baseOpts = {
    count: 0,
    getScrollElement: () => logContainer,
    estimateSize: () => ROW_HEIGHT,
    overscan: OVERSCAN,
    observeElementRect,
    observeElementOffset,
    scrollToFn: (offset, { adjustments = 0, behavior }, instance) => {
      const el = instance.scrollElement;
      if (!el) return;
      el.scrollTo({ top: offset + adjustments, left: el.scrollLeft, behavior });
    },
    onChange: () => scheduleRender(),
  };
  virtualizer = new Virtualizer(baseOpts);
  virtualizer._willUpdate();

  // Batch responses from worker
  on("batch", (msg) => {
    const start = msg.start;
    for (let i = 0; i < msg.entries.length; i++) {
      entryCache.set(start + i, msg.entries[i]);
    }
    pendingKey = null;
    scheduleRender();
  });

  // Data update from worker
  on("update", (msg) => {
    let filterChanged = false;
    if (msg.filterVersion !== lastFilterVersion) {
      entryCache.clear();
      pendingKey = null;
      lastFilterVersion = msg.filterVersion;
      lastRenderKey = "";
      filterChanged = true;
    }

    if (msg.total > 0 && msg.total - msg.indexed > 100) {
      const pct = ((msg.indexed / msg.total) * 100) | 0;
      statIndexed.textContent = `fts: ${pct}%`;
      statIndexed.classList.remove("hidden");
    } else {
      statIndexed.classList.add("hidden");
    }

    if (msg.dbSize > 0) {
      statDbSize.textContent = `db: ${fmtSize(msg.dbSize)}`;
      statDbSize.classList.remove("hidden");
    }

    virtualizer.setOptions({ ...baseOpts, count: ui.filteredLogs });
    logViewport.style.height = virtualizer.getTotalSize() + "px";
    emptyState.classList.toggle("hidden", ui.totalLogs > 0);
    updateCounts();

    if (pendingScrollToRaw >= 0 && filterChanged) {
      ui.dbWorker.postMessage({ type: "findPosition", rawIndex: pendingScrollToRaw });
      pendingScrollToRaw = -1;
    } else if (pendingScrollRestore && filterChanged) {
      const restore = pendingScrollRestore;
      pendingScrollRestore = null;
      logContainer.scrollTop = restore.scrollTop;
      logContainer.scrollLeft = restore.scrollLeft;
      syncAutoScroll();
      scheduleRender();
    } else if (ui.autoScroll) {
      scrollToBottom();
      scheduleRender();
    } else if (filterChanged) {
      syncAutoScroll();
      scheduleRender();
    }
  });

  // Worker resolved the sorted position of a raw index — scroll to it
  on("position", (msg) => {
    if (msg.position >= 0) {
      ui.autoScroll = false;
      updateAutoScrollBtn();
      virtualizer.scrollToIndex(msg.position, { align: "center" });
      scheduleRender();
    }
  });
}
