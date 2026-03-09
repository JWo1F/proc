// Cached DOM element references.

export const $ = (id) => document.getElementById(id);

export const logViewport = $("log-viewport");
export const logContainer = $("log-container");
export const emptyState = $("empty-state");
export const logCountEl = $("log-count");
export const statusBadge = $("status-badge");
export const filterCount = $("filter-count");

// Search
export const searchInput = $("search-input");
export const searchClear = $("search-clear");
export const searchBar = $("search-bar");
export const searchCaseBtn = $("search-case-btn");
export const searchWordBtn = $("search-word-btn");
export const searchRegexBtn = $("search-regex-btn");
export const searchError = $("search-error");

// Theme
export const themeToggle = $("theme-toggle");
export const themeIconLight = $("theme-icon-light");
export const themeIconDark = $("theme-icon-dark");

// Auto-scroll
export const autoscrollToggle = $("autoscroll-toggle");

// Process filter
export const processFilterBtn = $("process-filter-btn");
export const processFilterDropdown = $("process-filter-dropdown");

// Download
export const downloadBtn = $("download-btn");
export const downloadDropdown = $("download-dropdown");
export const downloadAll = $("download-all");
export const downloadFiltered = $("download-filtered");
