// Browser-like history for filter/search/correlation state.
// Each user action that changes filters pushes a snapshot.
// Back/forward navigates the stack and restores UI state.
//
// Note: circular imports with search.js and token-filter.js are safe because
// all cross-references are function calls that happen at runtime, after
// all modules have finished initializing.

import { ui } from "../main.js";
import {
  logContainer,
  searchInput,
  searchClear,
  searchBar,
  searchError,
} from "../lib/dom.js";
import { sendFilter, updateSearchToggles } from "./search.js";
import { renderAllLogs } from "./virtual-scroll.js";
import { rebuildProcessFilter } from "./process-filter.js";
import { rebuildLevelFilter } from "./level-filter.js";
import { renderTokenBar } from "./token-filter.js";

const stack = [];
let cursor = -1;
let restoring = false;

function snapshot() {
  return {
    searchQuery: ui.searchQuery,
    searchCaseSensitive: ui.searchCaseSensitive,
    searchWholeWord: ui.searchWholeWord,
    searchRegex: ui.searchRegex,
    hiddenProcesses: new Set(ui.hiddenProcesses),
    hiddenLevels: new Set(ui.hiddenLevels),
    activeTokens: new Set(ui.activeTokens),
    scrollLeft: logContainer.scrollLeft,
    scrollTop: logContainer.scrollTop,
  };
}

function statesEqual(a, b) {
  return (
    a.searchQuery === b.searchQuery &&
    a.searchCaseSensitive === b.searchCaseSensitive &&
    a.searchWholeWord === b.searchWholeWord &&
    a.searchRegex === b.searchRegex &&
    setsEqual(a.hiddenProcesses, b.hiddenProcesses) &&
    setsEqual(a.hiddenLevels, b.hiddenLevels) &&
    setsEqual(a.activeTokens, b.activeTokens)
  );
}

function setsEqual(a, b) {
  if (a.size !== b.size) return false;
  for (const v of a) if (!b.has(v)) return false;
  return true;
}

function saveScroll() {
  if (cursor >= 0 && cursor < stack.length) {
    stack[cursor].scrollLeft = logContainer.scrollLeft;
    stack[cursor].scrollTop = logContainer.scrollTop;
  }
}

function applyState(state) {
  restoring = true;
  saveScroll();

  ui.searchQuery = state.searchQuery;
  ui.searchCaseSensitive = state.searchCaseSensitive;
  ui.searchWholeWord = state.searchWholeWord;
  ui.searchRegex = state.searchRegex;
  ui.hiddenProcesses = new Set(state.hiddenProcesses);
  ui.hiddenLevels = new Set(state.hiddenLevels);
  ui.activeTokens = new Set(state.activeTokens);

  // Update UI controls
  searchInput.value = state.searchQuery;
  searchClear.classList.toggle("hidden", !state.searchQuery);
  searchError.classList.add("hidden");
  searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
  updateSearchToggles();
  rebuildProcessFilter();
  rebuildLevelFilter();
  renderTokenBar();

  sendFilter();
  ui.autoScroll = false;
  renderAllLogs();

  // Restore scroll after render settles
  requestAnimationFrame(() => {
    logContainer.scrollLeft = state.scrollLeft;
    logContainer.scrollTop = state.scrollTop;
    restoring = false;
  });
}

// ── Public API ──────────────────────────────────────────────────────

/** Push current state onto the history stack. Call after any filter change. */
export function pushHistory() {
  if (restoring) return;

  const state = snapshot();

  // Don't push duplicate states
  if (cursor >= 0 && statesEqual(stack[cursor], state)) {
    stack[cursor].scrollLeft = state.scrollLeft;
    stack[cursor].scrollTop = state.scrollTop;
    return;
  }

  saveScroll();

  // Truncate forward history
  stack.length = cursor + 1;
  stack.push(state);
  cursor = stack.length - 1;
}

/** Navigate back in history. */
export function historyBack() {
  if (cursor <= 0) return;
  cursor--;
  applyState(stack[cursor]);
}

/** Navigate forward in history. */
export function historyForward() {
  if (cursor >= stack.length - 1) return;
  cursor++;
  applyState(stack[cursor]);
}

export function initHistory() {
  // Push initial empty state
  pushHistory();

  // Keyboard shortcuts: Cmd+[/] on Mac, Alt+Left/Right elsewhere
  document.addEventListener("keydown", (e) => {
    const isMac = navigator.platform.includes("Mac");

    if (isMac) {
      if (e.metaKey && e.key === "[") {
        e.preventDefault();
        historyBack();
      } else if (e.metaKey && e.key === "]") {
        e.preventDefault();
        historyForward();
      }
    } else {
      if (e.altKey && e.key === "ArrowLeft") {
        e.preventDefault();
        historyBack();
      } else if (e.altKey && e.key === "ArrowRight") {
        e.preventDefault();
        historyForward();
      }
    }
  });
}
