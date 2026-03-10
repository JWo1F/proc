import { ui } from "../main.js";
import { levelFilterBtn, levelFilterDropdown } from "../lib/dom.js";
import { renderAllLogs } from "./virtual-scroll.js";
import { sendFilter } from "./search.js";
import { closeAllDropdowns, setupDropdown } from "./process-filter.js";

const LEVELS = [
  { key: "debug", label: "Debug", color: "#9ca3af" },
  { key: "info", label: "Info", color: "#3b82f6" },
  { key: "warn", label: "Warn", color: "#eab308" },
  { key: "error", label: "Error", color: "#ef4444" },
  { key: "fatal", label: "Fatal", color: "#dc2626" },
  { key: "none", label: "No level", color: "#6b7280" },
];

export function rebuildLevelFilter() {
  levelFilterDropdown.innerHTML = "";
  for (const { key, label, color } of LEVELS) {
    const btn = document.createElement("button");
    btn.className =
      "w-full text-left px-3 py-1.5 text-sm flex items-center gap-2 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors";

    const dot = document.createElement("span");
    dot.className = "w-2.5 h-2.5 rounded-full flex-none";
    dot.style.backgroundColor = color;

    const lbl = document.createElement("span");
    lbl.className = "flex-1 truncate";
    lbl.textContent = label;

    const check = document.createElement("span");
    check.className = "flex-none text-blue-500";
    check.innerHTML = ui.hiddenLevels.has(key) ? "" : "&#10003;";

    btn.appendChild(dot);
    btn.appendChild(lbl);
    btn.appendChild(check);

    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      if (ui.hiddenLevels.has(key)) {
        ui.hiddenLevels.delete(key);
      } else {
        ui.hiddenLevels.add(key);
      }
      sendFilter();
      rebuildLevelFilter();
      renderAllLogs();
    });

    levelFilterDropdown.appendChild(btn);
  }
}

export function initLevelFilter() {
  setupDropdown(levelFilterBtn, levelFilterDropdown);
  rebuildLevelFilter();
}
