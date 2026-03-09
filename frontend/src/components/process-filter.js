import { state } from "../lib/state.js";
import { processFilterBtn, processFilterDropdown, downloadBtn, downloadDropdown } from "../lib/dom.js";
import { renderAllLogs } from "./virtual-scroll.js";

function closeAllDropdowns() {
  processFilterDropdown.classList.add("hidden");
  downloadDropdown.classList.add("hidden");
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
  for (const [name, color] of state.knownProcesses) {
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
    check.innerHTML = state.hiddenProcesses.has(name) ? "" : "&#10003;";

    btn.appendChild(dot);
    btn.appendChild(label);
    btn.appendChild(check);

    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      if (state.hiddenProcesses.has(name)) {
        state.hiddenProcesses.delete(name);
      } else {
        state.hiddenProcesses.add(name);
      }
      rebuildProcessFilter();
      renderAllLogs();
    });

    processFilterDropdown.appendChild(btn);
  }
}

export function registerProcess(name, color) {
  if (state.knownProcesses.has(name)) return;
  state.knownProcesses.set(name, color);
  rebuildProcessFilter();
}

// Also exported for dropdown close from downloads
export { closeAllDropdowns };

export function initProcessFilter() {
  setupDropdown(processFilterBtn, processFilterDropdown);
  setupDropdown(downloadBtn, downloadDropdown);
  document.addEventListener("click", closeAllDropdowns);
}
