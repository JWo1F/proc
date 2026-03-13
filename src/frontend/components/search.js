import { ui } from "../main.js";
import {
  searchInput,
  searchClear,
  searchBar,
  searchError,
  tokenFilterBar,
  tokenFilterTags,
} from "../lib/dom.js";
import { renderAllLogs } from "./virtual-scroll.js";
import { rebuildProcessFilter } from "./process-filter.js";
import { rebuildLevelFilter } from "./level-filter.js";
import { pushHistory } from "./history.js";

function sendFilter() {
  ui.dbWorker.postMessage({
    type: "setFilter",
    query: ui.searchQuery,
    hiddenProcesses: Array.from(ui.hiddenProcesses),
    hiddenLevels: Array.from(ui.hiddenLevels),
    activeTokens: Array.from(ui.activeTokens),
  });
}

function applySearch() {
  ui.searchQuery = searchInput.value.trim();
  searchClear.classList.toggle("hidden", !ui.searchQuery);
  searchError.classList.add("hidden");
  searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
  sendFilter();
  renderAllLogs();
  pushHistory();
}

// Clear all filters (search, processes, levels, tokens) and notify worker
export function clearAllFilters() {
  searchInput.value = "";
  ui.searchQuery = "";
  ui.hiddenProcesses.clear();
  ui.hiddenLevels.clear();
  ui.activeTokens.clear();
  searchClear.classList.add("hidden");
  searchError.classList.add("hidden");
  searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
  tokenFilterBar.classList.add("hidden");
  tokenFilterTags.innerHTML = "";
  rebuildProcessFilter();
  rebuildLevelFilter();
  sendFilter();
  pushHistory();
}

// Exported so process-filter can also trigger filter updates
export { sendFilter };

// ── FTS syntax help dialog ──────────────────────────────────────────

const HELP_ROWS = [
  ["error", "Substring match (<em>error</em>, <em>stderr</em>, ...)"],
  ["error timeout", "Both terms (implicit AND)"],
  ["error AND timeout", "Both terms (explicit AND)"],
  ["error OR fatal", "Either term"],
  ["NOT debug", "Exclude lines with <em>debug</em>"],
  ["(err OR warn) AND db", "Grouped expression"],
  ['"connection reset"', "Exact phrase"],
  ["err*", "Prefix match (<em>error</em>, <em>errs</em>, ...)"],
  ["NEAR(error crash)", "Terms near each other (default 10 tokens)"],
  ["NEAR(error crash, 5)", "Terms within 5 tokens"],
  ["^ error", "Match at start of line"],
  ["plain:error", "Search log text column only"],
  ["process_name:web", "Search process name only"],
];

function createHelpDialog() {
  const dialog = document.createElement("dialog");
  dialog.className =
    "rounded-2xl shadow-2xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100 p-0 max-w-xl w-[calc(100%-2rem)]";

  const rows = HELP_ROWS.map(([q, desc], i) => {
    const border = i < HELP_ROWS.length - 1
      ? "border-b border-gray-100 dark:border-gray-800"
      : "";
    return `<tr class="${border}">
      <td class="py-2 pr-6"><code class="px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-blue-600 dark:text-blue-400 text-xs">${q}</code></td>
      <td class="py-2 text-gray-500 dark:text-gray-400">${desc}</td>
    </tr>`;
  }).join("");

  dialog.innerHTML = `
    <div class="px-6 pt-5 pb-2 flex items-start justify-between">
      <div>
        <h2 class="text-sm font-semibold tracking-tight">Search syntax</h2>
        <p class="text-xs text-gray-400 dark:text-gray-500 mt-0.5">FTS5 full-text search with trigram tokenizer</p>
      </div>
      <button data-close class="ml-4 p-1 -mr-1 -mt-0.5 rounded-lg text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors">
        <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
        </svg>
      </button>
    </div>
    <div class="px-6 pb-5">
      <table class="w-full text-[13px] leading-relaxed">
        <tbody>${rows}</tbody>
      </table>
      <p class="text-[11px] text-gray-400 dark:text-gray-500 mt-3 leading-relaxed">
        Case-insensitive &middot; Min 3 characters (trigram) &middot; Operators must be UPPERCASE
        &middot; Columns: <code class="px-1 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-[10px]">plain</code>
        <code class="px-1 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-[10px]">process_name</code>
      </p>
    </div>
  `;
  document.body.appendChild(dialog);

  dialog.querySelector("[data-close]").addEventListener("click", () => dialog.close());
  dialog.addEventListener("click", (e) => {
    if (e.target === dialog) dialog.close();
  });
  dialog.addEventListener("keydown", (e) => {
    if (e.key === "Escape") dialog.close();
  });

  return dialog;
}

let helpDialog = null;

function showHelp() {
  if (!helpDialog) helpDialog = createHelpDialog();
  helpDialog.showModal();
}

export function initSearch() {
  searchInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") applySearch();
  });

  searchInput.addEventListener("blur", () => {
    applySearch();
  });

  searchClear.addEventListener("click", () => {
    searchInput.value = "";
    ui.searchQuery = "";
    searchClear.classList.add("hidden");
    searchError.classList.add("hidden");
    searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
    sendFilter();
    renderAllLogs();
    pushHistory();
  });

  // Help button
  document.getElementById("search-help-btn").addEventListener("click", showHelp);

  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "f") {
      e.preventDefault();
      searchInput.focus();
      searchInput.select();
    }
    if (e.key === "Escape" && document.activeElement === searchInput) {
      searchInput.blur();
    }
  });
}
