// Shared dropdown builder for filter dropdowns.

import { toggleFilter } from "./filter-set.js";

export function buildFilterDropdown(container, items, hiddenSet, rebuildFn) {
  container.innerHTML = "";
  for (const { key, label, color } of items) {
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
    check.innerHTML = hiddenSet.has(key) ? "" : "&#10003;";

    btn.appendChild(dot);
    btn.appendChild(lbl);
    btn.appendChild(check);

    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleFilter(hiddenSet, key, rebuildFn);
    });

    container.appendChild(btn);
  }
}
