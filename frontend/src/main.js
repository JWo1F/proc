import "./style.css";

import { state } from "./lib/state.js";
import { initTheme } from "./components/theme.js";
import { initAutoScroll } from "./components/auto-scroll.js";
import { initSearch } from "./components/search.js";
import { initProcessFilter } from "./components/process-filter.js";
import { initDownloads } from "./components/downloads.js";
import { initVirtualScroll } from "./components/virtual-scroll.js";
import { connectSSE } from "./lib/sse.js";

initTheme();
initAutoScroll();
initSearch();
initProcessFilter();
initDownloads();
initVirtualScroll();
connectSSE();
