// Unified filter toggle logic — toggle/clear a Set then sync state.

import { sendFilter } from "../components/search.js";
import { renderAllLogs } from "../components/virtual-scroll.js";
import { pushHistory } from "../components/history.js";

export function toggleFilter(set, value, rebuildFn) {
  if (set.has(value)) set.delete(value);
  else set.add(value);
  rebuildFn();
  sendFilter();
  renderAllLogs();
  pushHistory();
}

export function clearFilter(set, rebuildFn) {
  set.clear();
  rebuildFn();
  sendFilter();
  renderAllLogs();
  pushHistory();
}
