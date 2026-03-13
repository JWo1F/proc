import { ui } from "../main.js";
import { autoscrollToggle, logContainer } from "../lib/dom.js";
import { scrollToBottom, renderVisible } from "./virtual-scroll.js";

export function updateAutoScrollBtn() {
  if (ui.autoScroll) {
    autoscrollToggle.className =
      "flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-lg border border-blue-300 dark:border-blue-600 bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400 hover:bg-blue-100 dark:hover:bg-blue-900/50 transition-colors";
  } else {
    autoscrollToggle.className =
      "flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-lg border border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800 text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors";
  }
}

/** Sync autoScroll flag with actual scroll position, then update button. */
export function syncAutoScroll() {
  const { scrollTop, scrollHeight, clientHeight } = logContainer;
  const atBottom = scrollHeight - scrollTop - clientHeight <= 40;
  ui.autoScroll = atBottom;
  updateAutoScrollBtn();
}

export function initAutoScroll() {
  let scrollDebounce = null;
  let programmaticScroll = false;

  window.__setProgrammaticScroll = (v) => {
    programmaticScroll = v;
  };

  autoscrollToggle.addEventListener("click", () => {
    ui.autoScroll = !ui.autoScroll;
    updateAutoScrollBtn();
    if (ui.autoScroll) scrollToBottom();
  });

  logContainer.addEventListener("scroll", () => {
    if (!programmaticScroll) {
      const { scrollTop, scrollHeight, clientHeight } = logContainer;
      const distFromBottom = scrollHeight - scrollTop - clientHeight;
      if (distFromBottom > 40 && ui.autoScroll) {
        ui.autoScroll = false;
        updateAutoScrollBtn();
      }
      if (distFromBottom <= 1 && !ui.autoScroll) {
        clearTimeout(scrollDebounce);
        scrollDebounce = setTimeout(() => {
          if (
            logContainer.scrollHeight -
              logContainer.scrollTop -
              logContainer.clientHeight <=
            1
          ) {
            ui.autoScroll = true;
            updateAutoScrollBtn();
          }
        }, 150);
      }
    }
    // renderVisible() already coalesces via scheduleRender/RAF
    renderVisible();
  });
}
