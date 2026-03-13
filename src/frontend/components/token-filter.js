// Token correlation filter — click tokens in log lines to filter
// across all processes. Manages the filter bar and click handling.

import { ui } from "../main.js";
import { toggleFilter, clearFilter } from "../lib/filter-set.js";
import {
  tokenFilterBar,
  tokenFilterTags,
  tokenFilterClear,
  logViewport,
} from "../lib/dom.js";

export function renderTokenBar() {
  const hasTokens = ui.activeTokens.size > 0;
  tokenFilterBar.classList.toggle("hidden", !hasTokens);

  tokenFilterTags.innerHTML = "";
  for (const token of ui.activeTokens) {
    const tag = document.createElement("span");
    tag.className =
      "inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-xs font-mono " +
      "bg-amber-200/60 dark:bg-amber-800/40 text-amber-800 dark:text-amber-200 " +
      "border border-amber-300 dark:border-amber-700";

    const label = document.createElement("span");
    label.textContent = token.length > 40 ? token.slice(0, 37) + "..." : token;
    label.title = token;

    const close = document.createElement("button");
    close.className =
      "text-amber-500 hover:text-amber-800 dark:hover:text-amber-100 leading-none";
    close.innerHTML = "&#x2715;";
    close.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleFilter(ui.activeTokens, token, renderTokenBar);
    });

    tag.appendChild(label);
    tag.appendChild(close);
    tokenFilterTags.appendChild(tag);
  }
}

export function initTokenFilter() {
  logViewport.addEventListener("click", (e) => {
    const span = e.target.closest(".log-token");
    if (!span) return;
    e.stopPropagation();
    const token = span.dataset.token;
    if (token) toggleFilter(ui.activeTokens, token, renderTokenBar);
  });

  tokenFilterClear.addEventListener("click", () => {
    clearFilter(ui.activeTokens, renderTokenBar);
  });
}
