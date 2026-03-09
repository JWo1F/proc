import { state } from "../lib/state.js";
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
  updateToggleBtn(searchCaseBtn, state.searchCaseSensitive);
  updateToggleBtn(searchWordBtn, state.searchWholeWord, " border-l border-gray-300 dark:border-gray-700");
  updateToggleBtn(searchRegexBtn, state.searchRegex, " border-l border-gray-300 dark:border-gray-700");
}

function buildSearchRegex() {
  if (!state.searchQuery) {
    state.compiledRegex = null;
    return;
  }
  try {
    const flags = state.searchCaseSensitive ? "g" : "gi";
    let pattern;
    if (state.searchRegex) {
      pattern = state.searchQuery;
    } else {
      pattern = state.searchQuery.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    }
    if (state.searchWholeWord) {
      pattern = `\\b${pattern}\\b`;
    }
    state.compiledRegex = new RegExp(pattern, flags);
    searchError.classList.add("hidden");
    searchError.title = "";
    searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
  } catch (e) {
    state.compiledRegex = null;
    searchError.textContent = e.message.replace("Invalid regular expression: ", "");
    searchError.title = e.message;
    searchError.classList.remove("hidden");
    searchBar.classList.add("!border-red-400", "dark:!border-red-500");
  }
}

function applySearch() {
  state.searchQuery = searchInput.value.trim();
  searchClear.classList.toggle("hidden", !state.searchQuery);
  buildSearchRegex();
  renderAllLogs();
}

export function initSearch() {
  let searchTimeout = null;

  searchInput.addEventListener("input", () => {
    clearTimeout(searchTimeout);
    searchTimeout = setTimeout(applySearch, 150);
  });

  searchClear.addEventListener("click", () => {
    searchInput.value = "";
    state.searchQuery = "";
    state.compiledRegex = null;
    searchClear.classList.add("hidden");
    searchError.classList.add("hidden");
    searchBar.classList.remove("!border-red-400", "dark:!border-red-500");
    renderAllLogs();
  });

  searchCaseBtn.addEventListener("click", () => {
    state.searchCaseSensitive = !state.searchCaseSensitive;
    updateSearchToggles();
    applySearch();
  });

  searchWordBtn.addEventListener("click", () => {
    state.searchWholeWord = !state.searchWholeWord;
    updateSearchToggles();
    applySearch();
  });

  searchRegexBtn.addEventListener("click", () => {
    state.searchRegex = !state.searchRegex;
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
