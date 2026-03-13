import { ui } from "../main.js";
import { levelFilterBtn, levelFilterDropdown } from "../lib/dom.js";
import { setupDropdown } from "./process-filter.js";
import { buildFilterDropdown } from "../lib/dropdown.js";

const LEVELS = [
  { key: "debug", label: "Debug", color: "#9ca3af" },
  { key: "info", label: "Info", color: "#3b82f6" },
  { key: "warn", label: "Warn", color: "#eab308" },
  { key: "error", label: "Error", color: "#ef4444" },
  { key: "fatal", label: "Fatal", color: "#dc2626" },
  { key: "none", label: "No level", color: "#6b7280" },
];

export function rebuildLevelFilter() {
  buildFilterDropdown(
    levelFilterDropdown,
    LEVELS,
    ui.hiddenLevels,
    rebuildLevelFilter,
  );
}

export function initLevelFilter() {
  setupDropdown(levelFilterBtn, levelFilterDropdown);
  rebuildLevelFilter();
}
