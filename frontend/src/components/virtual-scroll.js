// Block-based virtual scrolling — requests batches from the worker,
// renders only visible blocks.

import { ui, onWorkerMessage } from "../main.js";
import { logViewport, logContainer, emptyState, logCountEl, filterCount, downloadFiltered } from "../lib/dom.js";

const LEVEL_LETTERS = { debug: "D", info: "I", warn: "W", error: "E", fatal: "F" };

const ROW_HEIGHT = 24;
const BLOCK_SIZE = 100;
const OVERSCAN = 30;

const renderedBlocks = new Map();   // blockIndex → DOM element
const pendingBlocks = new Set();    // blocks we've requested but not received

export function clearBlocks() {
  for (const el of renderedBlocks.values()) el.remove();
  renderedBlocks.clear();
  pendingBlocks.clear();
}

function createLogElement(entry) {
  const div = document.createElement("div");
  let cls = "log-line flex items-start gap-3 px-2 py-0.5 rounded font-mono text-[13px] leading-relaxed";
  if (entry.level) cls += ` level-${entry.level}`;
  div.className = cls;

  const idx = document.createElement("span");
  idx.className = "flex-none w-12 text-right text-gray-400 dark:text-gray-600 select-none text-xs leading-relaxed cursor-pointer";
  idx.textContent = "0x" + entry.index.toString(16).toUpperCase();

  const ts = document.createElement("span");
  ts.className = "flex-none w-16 text-gray-400 dark:text-gray-500 text-xs leading-relaxed";
  ts.textContent = entry.timestamp;

  const proc = document.createElement("span");
  proc.className = "flex-none w-20 truncate text-xs font-medium leading-relaxed";
  proc.style.color = entry.color;
  proc.textContent = entry.process;

  const lvl = document.createElement("span");
  lvl.className = "flex-none w-4 text-center text-xs font-bold leading-relaxed level-badge level-badge-" + (entry.level || "none");
  lvl.textContent = LEVEL_LETTERS[entry.level] || "-";

  const content = document.createElement("span");
  content.className = "flex-1 min-w-0 leading-relaxed whitespace-pre overflow-hidden text-ellipsis";
  if (entry.system) {
    content.classList.add("font-medium");
    content.style.color = entry.color;
  }
  content.innerHTML = entry.html;

  // Click index to expand/collapse long lines
  idx.addEventListener("click", (e) => {
    e.stopPropagation();
    div.classList.toggle("expanded");
    if (div.classList.contains("expanded")) {
      content.classList.remove("overflow-hidden", "text-ellipsis", "whitespace-pre");
      content.classList.add("whitespace-pre-wrap", "break-all");
    } else {
      content.classList.add("overflow-hidden", "text-ellipsis", "whitespace-pre");
      content.classList.remove("whitespace-pre-wrap", "break-all");
    }
  });

  div.appendChild(idx);
  div.appendChild(ts);
  div.appendChild(proc);
  div.appendChild(lvl);
  div.appendChild(content);

  return div;
}

function createBlockFromEntries(entries, blockIdx) {
  const el = document.createElement("div");
  el.style.cssText = "position:absolute;left:0;right:0;top:" + (blockIdx * BLOCK_SIZE * ROW_HEIGHT) + "px";
  for (const entry of entries) {
    el.appendChild(createLogElement(entry));
  }
  return el;
}

function updateCounts() {
  logCountEl.textContent = `${ui.totalLogs} lines`;
  const hasFilter = ui.searchQuery || ui.hiddenProcesses.size > 0 || ui.activeTokens.size > 0;
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

export function renderVisible() {
  const totalRows = ui.filteredLogs;
  logViewport.style.height = totalRows * ROW_HEIGHT + "px";
  emptyState.classList.toggle("hidden", ui.totalLogs > 0);

  if (totalRows === 0) {
    clearBlocks();
    updateCounts();
    return;
  }

  const scrollTop = logContainer.scrollTop;
  const viewportHeight = logContainer.clientHeight;

  let startRow = Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN;
  let endRow = Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN;
  startRow = Math.max(0, startRow);
  endRow = Math.min(totalRows, endRow);

  if (endRow <= 0) {
    clearBlocks();
    updateCounts();
    return;
  }

  const startBlock = Math.floor(startRow / BLOCK_SIZE);
  const endBlock = Math.floor((endRow - 1) / BLOCK_SIZE);

  // Remove blocks and pending requests out of range
  for (const [idx, el] of renderedBlocks) {
    if (idx < startBlock || idx > endBlock) {
      el.remove();
      renderedBlocks.delete(idx);
    }
  }
  for (const idx of pendingBlocks) {
    if (idx < startBlock || idx > endBlock) {
      pendingBlocks.delete(idx);
    }
  }

  // Request blocks in range
  for (let b = startBlock; b <= endBlock; b++) {
    if (renderedBlocks.has(b) || pendingBlocks.has(b)) continue;
    pendingBlocks.add(b);
    ui.worker.postMessage({
      type: "getBatch",
      id: b,
      start: b * BLOCK_SIZE,
      count: BLOCK_SIZE,
    });
  }

  updateCounts();
}

export function renderAllLogs() {
  clearBlocks();
  renderVisible();
  if (ui.autoScroll) scrollToBottom();
}

export function scrollToBottom() {
  window.__setProgrammaticScroll?.(true);
  logContainer.scrollTop = logContainer.scrollHeight;
  requestAnimationFrame(() => {
    window.__setProgrammaticScroll?.(false);
  });
}

export function initVirtualScroll() {
  // Handle batch responses from worker
  onWorkerMessage("batch", (msg) => {
    const blockIdx = msg.id;
    pendingBlocks.delete(blockIdx);

    // Ignore if already rendered or no longer needed
    if (renderedBlocks.has(blockIdx)) return;

    const el = createBlockFromEntries(msg.entries, blockIdx);
    logViewport.appendChild(el);
    renderedBlocks.set(blockIdx, el);
  });

  // Handle update notifications — new data available
  onWorkerMessage("update", () => {
    const prevFiltered = ui.filteredLogs;

    // Invalidate the last block (it may have grown)
    if (prevFiltered > 0) {
      const lastBlock = Math.floor((prevFiltered - 1) / BLOCK_SIZE);
      const el = renderedBlocks.get(lastBlock);
      if (el) { el.remove(); renderedBlocks.delete(lastBlock); }
      pendingBlocks.delete(lastBlock);
    }

    logViewport.style.height = ui.filteredLogs * ROW_HEIGHT + "px";
    emptyState.classList.toggle("hidden", ui.totalLogs > 0);

    if (ui.autoScroll) scrollToBottom();
    renderVisible();
  });

  // Handle init event
  onWorkerMessage("init", (msg) => {
    ui.projectName = msg.name;
    document.title = `Procfile: ${msg.name}`;
    document.getElementById("app-title").textContent = msg.name;
    document.getElementById("app-addr").textContent = location.host;
  });

  // Handle connection status
  onWorkerMessage("connected", (msg) => {
    const badge = document.getElementById("status-badge");
    if (msg.value) {
      badge.className = "flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-400";
      badge.innerHTML = '<span class="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse"></span><span>connected</span>';
    } else {
      badge.className = "flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400";
      badge.innerHTML = '<span class="w-1.5 h-1.5 rounded-full bg-red-500"></span><span>disconnected</span>';
    }
  });
}
