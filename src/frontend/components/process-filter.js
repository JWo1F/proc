import { ui, onWorkerMessage } from "../main.js";
import {
  processFilterBtn,
  processFilterDropdown,
  downloadBtn,
  downloadDropdown,
  levelFilterDropdown,
} from "../lib/dom.js";
import { renderAllLogs } from "./virtual-scroll.js";
import { sendFilter } from "./search.js";
import { pushHistory } from "./history.js";

function closeAllDropdowns() {
  processFilterDropdown.classList.add("hidden");
  downloadDropdown.classList.add("hidden");
  levelFilterDropdown.classList.add("hidden");
}

function setupDropdown(btn, dropdown) {
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    const isVisible = !dropdown.classList.contains("hidden");
    closeAllDropdowns();
    if (!isVisible) dropdown.classList.remove("hidden");
  });
}

function rebuildProcessFilter() {
  processFilterDropdown.innerHTML = "";
  for (const [name, color] of ui.processes) {
    const btn = document.createElement("button");
    btn.className =
      "w-full text-left px-3 py-1.5 text-sm flex items-center gap-2 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors";

    const dot = document.createElement("span");
    dot.className = "w-2.5 h-2.5 rounded-full flex-none";
    dot.style.backgroundColor = color;

    const label = document.createElement("span");
    label.className = "flex-1 truncate";
    label.textContent = name;

    const check = document.createElement("span");
    check.className = "flex-none text-blue-500";
    check.innerHTML = ui.hiddenProcesses.has(name) ? "" : "&#10003;";

    btn.appendChild(dot);
    btn.appendChild(label);
    btn.appendChild(check);

    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      if (ui.hiddenProcesses.has(name)) {
        ui.hiddenProcesses.delete(name);
      } else {
        ui.hiddenProcesses.add(name);
      }
      sendFilter();
      rebuildProcessFilter();
      renderAllLogs();
      pushHistory();
    });

    processFilterDropdown.appendChild(btn);
  }
}

export { closeAllDropdowns, setupDropdown, rebuildProcessFilter };

export function initProcessFilter() {
  setupDropdown(processFilterBtn, processFilterDropdown);
  setupDropdown(downloadBtn, downloadDropdown);
  document.addEventListener("click", closeAllDropdowns);

  // Rebuild filter dropdown when processes change
  onWorkerMessage("update", () => {
    rebuildProcessFilter();
  });
}
