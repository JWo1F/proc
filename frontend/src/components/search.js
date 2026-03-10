import { ui } from "../main.js";
import { searchInput, searchClear, searchBar, searchCaseBtn, searchWordBtn, searchRegexBtn, searchError } from "../lib/dom.js";
import { renderAllLogs } from "./virtual-scroll.js";

const TOGGLE_ACTIVE =
  "px-2 h-full text-xs font-bold text-blue-600 dark:text-blue-400 bg-blue-100 dark:bg-blue-900/40 hover:bg-blue-200 dark:hover:bg-blue-800/50 transition-colors leading-none";
const TOGGLE_INACTIVE =
  "px-2 h-full text-xs font-bold text-gray-400 dark:text-gray-500 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors leading-none";

function updateToggleBtn(btn, active, extraClass) {
  btn.className = (active ? TOGGLE_ACTIVE : TOGGLE_INACTIVE) + (extraClass || "");
}

function updateSearchToggles() {
  updateToggleBtn(searchCaseBtn, ui.searchCaseSensitive);
  updateToggleBtn(searchWordBtn, ui.searchWholeWord, " border-l border-gray-300 dark:border-gray-700");
  updateToggleBtn(searchRegexBtn, ui.searchRegex, " border-l border-gray-300 dark:border-gray-700");
}

function validateRegex() {
  if (!ui.searchQuery || !ui.searchRegex) {
    searchError.classList.add("hidden");
    searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
    return;
  }
  try {
    new RegExp(ui.searchQuery);
    searchError.classList.add("hidden");
    searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
  } catch (e) {
    searchError.textContent = e.message.replace("Invalid regular expression: ", "");
    searchError.title = e.message;
    searchError.classList.remove("hidden");
    searchBar.classList.add("!border-red-400", "dark:!border-red-500");
  }
}

function sendFilter() {
  ui.worker.postMessage({
    type: "setFilter",
    query: ui.searchQuery,
    caseSensitive: ui.searchCaseSensitive,
    wholeWord: ui.searchWholeWord,
    regex: ui.searchRegex,
    hiddenProcesses: Array.from(ui.hiddenProcesses),
    hiddenLevels: Array.from(ui.hiddenLevels),
  });
}

function applySearch() {
  ui.searchQuery = searchInput.value.trim();
  searchClear.classList.toggle("hidden", !ui.searchQuery);
  validateRegex();
  sendFilter();
  renderAllLogs();
}

// Exported so process-filter can also trigger filter updates
export { sendFilter };

export function initSearch() {
  let searchTimeout = null;

  searchInput.addEventListener("input", () => {
    clearTimeout(searchTimeout);
    searchTimeout = setTimeout(applySearch, 150);
  });

  searchClear.addEventListener("click", () => {
    searchInput.value = "";
    ui.searchQuery = "";
    searchClear.classList.add("hidden");
    searchError.classList.add("hidden");
    searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
    sendFilter();
    renderAllLogs();
  });

  searchCaseBtn.addEventListener("click", () => {
    ui.searchCaseSensitive = !ui.searchCaseSensitive;
    updateSearchToggles();
    applySearch();
  });

  searchWordBtn.addEventListener("click", () => {
    ui.searchWholeWord = !ui.searchWholeWord;
    updateSearchToggles();
    applySearch();
  });

  searchRegexBtn.addEventListener("click", () => {
    ui.searchRegex = !ui.searchRegex;
    updateSearchToggles();
    applySearch();
  });

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
