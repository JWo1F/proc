import { state } from "../lib/state.js";
import { autoscrollToggle, logContainer } from "../lib/dom.js";
import { scrollToBottom, renderVisible } from "./virtual-scroll.js";

function updateAutoScrollBtn() {
  if (state.autoScroll) {
    autoscrollToggle.className =
      "flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-lg border border-blue-300 dark:border-blue-600 bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400 hover:bg-blue-100 dark:hover:bg-blue-900/50 transition-colors";
  } else {
    autoscrollToggle.className =
      "flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-lg border border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800 text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors";
  }
}

export function initAutoScroll() {
  let scrollDebounce = null;
  let programmaticScroll = false;
  let scrollRafId = null;

  // Expose programmatic scroll flag for scrollToBottom
  window.__setProgrammaticScroll = (v) => { programmaticScroll = v; };

  autoscrollToggle.addEventListener("click", () => {
    state.autoScroll = !state.autoScroll;
    updateAutoScrollBtn();
    if (state.autoScroll) scrollToBottom();
  });

  logContainer.addEventListener("scroll", () => {
    if (!programmaticScroll) {
      const { scrollTop, scrollHeight, clientHeight } = logContainer;
      const distFromBottom = scrollHeight - scrollTop - clientHeight;
      if (distFromBottom > 40 && state.autoScroll) {
        state.autoScroll = false;
        updateAutoScrollBtn();
      }
      if (distFromBottom <= 1 && !state.autoScroll) {
        clearTimeout(scrollDebounce);
        scrollDebounce = setTimeout(() => {
          if (logContainer.scrollHeight - logContainer.scrollTop - logContainer.clientHeight <= 1) {
            state.autoScroll = true;
            updateAutoScrollBtn();
          }
        }, 150);
      }
    }
    if (scrollRafId === null) {
      scrollRafId = requestAnimationFrame(() => {
        scrollRafId = null;
        renderVisible();
      });
    }
  });
}
