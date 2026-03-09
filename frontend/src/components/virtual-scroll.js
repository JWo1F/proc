// Block-based virtual scrolling — renders only visible blocks of rows.

import { state } from "../lib/state.js";
import { logViewport, logContainer, emptyState, logCountEl, filterCount, downloadFiltered } from "../lib/dom.js";
import { createLogElement } from "../lib/log-entry.js";
import { rebuildFilteredLogs } from "../lib/filters.js";

const ROW_HEIGHT = 24;
const BLOCK_SIZE = 100;
const OVERSCAN = 30;

export function clearBlocks() {
  for (const el of state.renderedBlocks.values()) el.remove();
  state.renderedBlocks.clear();
}

function createBlock(blockIdx) {
  const el = document.createElement("div");
  el.style.cssText =
    "position:absolute;left:0;right:0;top:" +
    blockIdx * BLOCK_SIZE * ROW_HEIGHT + "px";

  const rowStart = blockIdx * BLOCK_SIZE;
  const rowEnd = Math.min(rowStart + BLOCK_SIZE, state.filteredLogs.length);

  for (let i = rowStart; i < rowEnd; i++) {
    el.appendChild(createLogElement(state.allLogs[state.filteredLogs[i]], false));
  }

  return el;
}

function updateCounts(visibleOverride) {
  const total = state.allLogs.length;
  const visible = visibleOverride !== undefined ? visibleOverride : state.filteredLogs.length;
  logCountEl.textContent = `${total} lines`;

  const hasFilter = state.searchQuery || state.hiddenProcesses.size > 0;
  if (hasFilter && visible !== total) {
    filterCount.textContent = `${visible} / ${total}`;
    filterCount.classList.remove("hidden");
    downloadFiltered.classList.remove("hidden");
    downloadFiltered.textContent = `Download filtered (${visible})`;
  } else {
    filterCount.classList.add("hidden");
    downloadFiltered.classList.add("hidden");
  }
}

export function renderVisible() {
  const totalRows = state.filteredLogs.length;
  logViewport.style.height = totalRows * ROW_HEIGHT + "px";
  emptyState.classList.toggle("hidden", state.allLogs.length > 0);

  if (totalRows === 0) {
    clearBlocks();
    updateCounts(0);
    return;
  }

  const scrollTop = logContainer.scrollTop;
  const viewportHeight = logContainer.clientHeight;

  let startRow = Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN;
  let endRow = Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN;
  startRow = Math.max(0, startRow);
  endRow = Math.min(totalRows, endRow);

  const startBlock = Math.floor(startRow / BLOCK_SIZE);
  const endBlock = Math.floor((endRow - 1) / BLOCK_SIZE);

  for (const [idx, el] of state.renderedBlocks) {
    if (idx < startBlock || idx > endBlock) {
      el.remove();
      state.renderedBlocks.delete(idx);
    }
  }

  for (let b = startBlock; b <= endBlock; b++) {
    if (state.renderedBlocks.has(b)) continue;
    const el = createBlock(b);
    logViewport.appendChild(el);
    state.renderedBlocks.set(b, el);
  }

  updateCounts(state.filteredLogs.length);
}

export function renderAllLogs() {
  rebuildFilteredLogs();
  clearBlocks();
  renderVisible();
  if (state.autoScroll) scrollToBottom();
}

export function scrollToBottom() {
  window.__setProgrammaticScroll?.(true);
  logContainer.scrollTop = logContainer.scrollHeight;
  requestAnimationFrame(() => {
    window.__setProgrammaticScroll?.(false);
  });
}

export function initVirtualScroll() {
  // Virtual scroll is initialized via auto-scroll's scroll listener
}
