// Shared application state — single source of truth for all modules.

export const state = {
  allLogs: [],
  filteredLogs: [],  // indices into allLogs matching current filter
  pendingEntries: [],

  autoScroll: true,
  projectName: "procfile",

  // Search
  searchQuery: "",
  searchCaseSensitive: false,
  searchWholeWord: false,
  searchRegex: false,
  compiledRegex: null,

  // Processes
  knownProcesses: new Map(),   // name -> color
  hiddenProcesses: new Set(),

  // Virtual scroll
  renderedBlocks: new Map(),   // blockIndex -> DOM element

  // SSE
  eventSource: null,
};
