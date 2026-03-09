// Log filtering logic — determines which entries are visible.

import { state } from "./state.js";

export function matchesFilter(entry) {
  if (state.hiddenProcesses.has(entry.process)) return false;
  if (!state.searchQuery) return true;
  if (!state.compiledRegex) return true; // invalid regex shows all
  state.compiledRegex.lastIndex = 0;
  if (state.compiledRegex.test(entry.line)) return true;
  state.compiledRegex.lastIndex = 0;
  if (state.compiledRegex.test(entry.process)) return true;
  return false;
}

export function rebuildFilteredLogs() {
  state.filteredLogs = [];
  for (let i = 0; i < state.allLogs.length; i++) {
    if (matchesFilter(state.allLogs[i])) {
      state.filteredLogs.push(i);
    }
  }
}
