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

// ── State ──────────────────────────────────────────────────────────────

export const entryCache = new Map(); // filteredIndex → entry
export const expandedLines = new Set(); // raw log indices of expanded lines

let lastFilterVersion = -1;
let pendingKey = null;
let pendingScrollRestore = null;
let selectedRawIndex = -1;
let pendingScrollToRaw = -1;

let contentEl = null;
let virtualizer = null;
let baseOpts = null;
let renderScheduled = false;
let measuring = false;
let contentDirty = false;

// Keyed pool: filteredIndex → DOM element.
// Elements stay in the DOM across renders; only their transform updates
// on scroll. Created when entering the visible range, removed on exit.
const pool = new Map();

// ── Expand/collapse toggle ────────────────────────────────────────────

export function toggleExpand(rawIndex) {
  if (expandedLines.has(rawIndex)) expandedLines.delete(rawIndex);
  else expandedLines.add(rawIndex);
  ui.autoScroll = false;
  updateAutoScrollBtn();
  contentDirty = true;
  render();
}

// ── HTML builders ─────────────────────────────────────────────────────

function buildRowClass(entry) {
  let cls =
    "log-line flex items-start gap-3 px-2 py-0.5 rounded font-mono text-[13px] leading-relaxed";
  if (entry.level) cls += ` level-${entry.level}`;
  if (expandedLines.has(entry.index)) cls += " expanded";
  if (entry.index === selectedRawIndex) cls += " log-line-selected";
  return cls;
}

function buildRowHTML(entry, sortedPos) {
  const expanded = expandedLines.has(entry.index);
  const sysFm = entry.system ? " font-medium" : "";
  const sysColor = entry.system ? ` style="color:${entry.color}"` : "";
  const wsPre = expanded ? "whitespace-pre-wrap break-all" : "whitespace-pre";

  return (
    `<span class="log-idx flex-none w-12 text-right text-gray-400 dark:text-gray-600 select-none text-xs leading-relaxed cursor-pointer">0x${sortedPos.toString(16).toUpperCase()}</span>` +
    `<span class="log-ts flex-none w-[5.5rem] text-gray-400 dark:text-gray-500 text-xs leading-relaxed cursor-pointer hover:text-blue-500 dark:hover:text-blue-400">${entry.timestamp}</span>` +
    `<span class="flex-none w-20 truncate text-xs font-medium leading-relaxed" style="color:${entry.color}">${entry.process}</span>` +
    `<span class="flex-none w-4 text-center text-xs font-bold leading-relaxed level-badge level-badge-${entry.level || "none"}">${LEVEL_LETTERS[entry.level] || "-"}</span>` +
    `<span class="flex-1 min-w-0 leading-relaxed ${wsPre}${sysFm}"${sysColor}>${entry.html}</span>`
  );
}

function createEl(entry, sortedPos) {
  const el = document.createElement("div");
  el.className = buildRowClass(entry);
  el.innerHTML = buildRowHTML(entry, sortedPos);
  el.setAttribute("data-index", sortedPos);
  el.style.cssText = "position:absolute;top:0;left:0;min-width:100%";
  return el;
}

// ── Event delegation ──────────────────────────────────────────────────

function initDelegation() {
  contentEl.addEventListener("click", (e) => {
    const idx = e.target.closest(".log-idx");
    if (idx) {
      e.stopPropagation();
      const row = idx.closest(".log-line");
      const entry = entryForRow(row);
      if (entry) toggleExpand(entry.index);
      return;
    }

    const ts = e.target.closest(".log-ts");
    if (ts) {
      e.stopPropagation();
      const row = ts.closest(".log-line");
      const entry = entryForRow(row);
      if (!entry) return;
      if (selectedRawIndex === entry.index) {
        selectedRawIndex = -1;
        contentDirty = true;
        render();
        return;
      }
      selectedRawIndex = entry.index;
      ui.autoScroll = false;
      updateAutoScrollBtn();
      pendingScrollToRaw = entry.index;
      clearAllFilters();
      renderAllLogs();
    }
  });
}

function entryForRow(row) {
  if (!row) return null;
  const idx = parseInt(row.getAttribute("data-index"), 10);
  return entryCache.get(idx) || null;
}

// ── Counts ─────────────────────────────────────────────────────────────

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

// ── Cache miss → worker request ────────────────────────────────────────

function requestMissing(items) {
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
}

// ── Pool helpers ───────────────────────────────────────────────────────

function clearPool() {
  for (const el of pool.values()) el.remove();
  pool.clear();
}

// ── Render ──────────────────────────────────────────────────────────────

function scheduleRender() {
  if (renderScheduled || measuring) return;
  renderScheduled = true;
  requestAnimationFrame(() => {
    renderScheduled = false;
    render();
  });
}

function render() {
  if (!virtualizer || ui.filteredLogs === 0) {
    clearPool();
    return;
  }

  virtualizer._willUpdate();
  const items = virtualizer.getVirtualItems();
  if (items.length === 0) {
    clearPool();
    return;
  }

  requestMissing(items);

  let cached = 0;
  let needsMeasure = false;

  for (const item of items) {
    const entry = entryCache.get(item.index);
    if (!entry) continue;
    cached++;

    let el = pool.get(item.index);
    if (el) {
      if (contentDirty) {
        el.className = buildRowClass(entry);
        el.innerHTML = buildRowHTML(entry, item.index);
      }
    } else {
      el = createEl(entry, item.index);
      pool.set(item.index, el);
      contentEl.appendChild(el);
    }

    el.style.transform = `translateY(${item.start}px)`;
    if (expandedLines.has(entry.index)) needsMeasure = true;
  }
  contentDirty = false;

  // Nothing cached yet — keep old pool elements as placeholders so the
  // viewport doesn't go blank while waiting for the worker batch.
  if (cached === 0) return;

  // Remove pool elements outside the virtual range. Elements inside the
  // range whose cache entry hasn't arrived yet are kept — they'll be
  // replaced once the batch comes in.
  const lo = items[0].index;
  const hi = items[items.length - 1].index;
  const savedLeft = logContainer.scrollLeft;
  for (const [idx, el] of pool) {
    if (idx < lo || idx > hi) {
      el.remove();
      pool.delete(idx);
    }
  }
  if (logContainer.scrollLeft !== savedLeft) {
    logContainer.scrollLeft = savedLeft;
  }

  logViewport.style.height = virtualizer.getTotalSize() + "px";

  // Measure expanded (variable-height) elements
  if (needsMeasure) {
    measuring = true;
    for (const el of contentEl.children) {
      virtualizer.measureElement(el);
    }
    measuring = false;
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
  clearPool();
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

// ── Init ───────────────────────────────────────────────────────────────

export function initVirtualScroll() {
  contentEl = document.createElement("div");
  contentEl.style.cssText = "position:absolute;top:0;left:0;min-width:100%";
  logViewport.appendChild(contentEl);
  initDelegation();

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
  virtualizer._didMount();
  virtualizer._willUpdate();

  // Batch responses from worker
  on("batch", (msg) => {
    for (let i = 0; i < msg.entries.length; i++) {
      entryCache.set(msg.start + i, msg.entries[i]);
    }
    pendingKey = null;
    scheduleRender();
  });

  // Data update from worker
  on("update", (msg) => {
    let filterChanged = false;
    if (msg.filterVersion !== lastFilterVersion) {
      pendingKey = null;
      lastFilterVersion = msg.filterVersion;
      filterChanged = true;
    }

    // Populate cache with piggybacked tail entries so auto-scroll
    // renders are instant — no getBatch round-trip needed.
    if (msg.tail) {
      for (let i = 0; i < msg.tail.length; i++) {
        entryCache.set(msg.tailStart + i, msg.tail[i]);
      }
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

    const savedTop = logContainer.scrollTop;
    const savedLeft = logContainer.scrollLeft;

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
    } else {
      logContainer.scrollTop = savedTop;
      logContainer.scrollLeft = savedLeft;
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
