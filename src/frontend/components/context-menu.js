// Row context menu — right-click on a log line for quick actions.

import { closeActive, showPopup } from "../lib/popup.js";
import { toggleFilter } from "../lib/filter-set.js";
import { logViewport } from "../lib/dom.js";
import { ui } from "../main.js";
import { entryCache, expandedLines, toggleExpand, renderAllLogs } from "./virtual-scroll.js";
import { rebuildProcessFilter } from "./process-filter.js";
import { renderTokenBar } from "./token-filter.js";
import { sendFilter } from "./search.js";
import { pushHistory } from "./history.js";

function buildMenu(entry) {
  const menu = document.createElement("div");
  menu.className =
    "bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-xl py-1 min-w-[180px] text-sm";

  const items = [];

  // Copy line (plain text)
  items.push({
    label: "Copy line",
    action: () => navigator.clipboard.writeText(entry.line),
  });

  // Filter by process — hide all others
  items.push({
    label: `Filter by "${entry.process}"`,
    action: () => {
      ui.hiddenProcesses.clear();
      for (const [name] of ui.processes) {
        if (name !== entry.process) ui.hiddenProcesses.add(name);
      }
      rebuildProcessFilter();
      sendFilter();
      renderAllLogs();
      pushHistory();
    },
  });

  // Filter by token — if line contains a detected token
  const tokenMatch = entry.html.match(/data-token="([^"]+)"/);
  if (tokenMatch) {
    const token = tokenMatch[1]
      .replace(/&amp;/g, "&")
      .replace(/&quot;/g, '"')
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">");
    items.push({
      label: "Filter by token",
      action: () => {
        toggleFilter(ui.activeTokens, token, renderTokenBar);
      },
    });
  }

  // Expand / Collapse
  const isExpanded = expandedLines.has(entry.index);
  items.push({
    label: isExpanded ? "Collapse line" : "Expand line",
    action: () => toggleExpand(entry.index),
  });

  for (const item of items) {
    const btn = document.createElement("button");
    btn.className =
      "w-full text-left px-3 py-1.5 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors text-gray-700 dark:text-gray-300";
    btn.textContent = item.label;
    btn.addEventListener("click", () => {
      closeActive();
      item.action();
    });
    menu.appendChild(btn);
  }

  return menu;
}

export function initContextMenu() {
  logViewport.addEventListener("contextmenu", (e) => {
    const row = e.target.closest(".log-line");
    if (!row) return;
    e.preventDefault();

    const idx = parseInt(row.getAttribute("data-index"), 10);
    const entry = entryCache.get(idx);
    if (!entry) return;

    const menu = buildMenu(entry);
    showPopup(null, menu, { anchor: "cursor", x: e.clientX, y: e.clientY });
  });
}
