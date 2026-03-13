import { ui, on } from "../main.js";
import {
  processFilterBtn,
  processFilterDropdown,
  downloadBtn,
  downloadDropdown,
  levelFilterDropdown,
} from "../lib/dom.js";
import { buildFilterDropdown } from "../lib/dropdown.js";

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
  const items = [...ui.processes].map(([name, color]) => ({
    key: name,
    label: name,
    color,
  }));
  buildFilterDropdown(
    processFilterDropdown,
    items,
    ui.hiddenProcesses,
    rebuildProcessFilter,
  );
}

export { closeAllDropdowns, setupDropdown, rebuildProcessFilter };

export function initProcessFilter() {
  setupDropdown(processFilterBtn, processFilterDropdown);
  setupDropdown(downloadBtn, downloadDropdown);
  document.addEventListener("click", closeAllDropdowns);

  on("update", () => {
    rebuildProcessFilter();
  });
}
