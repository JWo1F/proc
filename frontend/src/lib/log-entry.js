// Creates DOM elements for individual log rows.

import { state } from "./state.js";
import { highlightText } from "./html.js";

export function createLogElement(entry, animate) {
  const div = document.createElement("div");
  div.className =
    "log-line flex items-start gap-3 px-2 py-0.5 rounded font-mono text-[13px] leading-relaxed" +
    (animate ? " fade-in" : "");
  div.dataset.index = entry.index;
  div.dataset.process = entry.process;

  const idx = document.createElement("span");
  idx.className =
    "flex-none w-20 text-right text-gray-400 dark:text-gray-600 select-none text-xs leading-relaxed";
  idx.textContent = "0x" + entry.index.toString(16).toUpperCase();

  const ts = document.createElement("span");
  ts.className =
    "flex-none w-16 text-gray-400 dark:text-gray-500 text-xs leading-relaxed";
  ts.textContent = entry.timestamp;

  const proc = document.createElement("span");
  proc.className =
    "flex-none w-20 truncate text-xs font-medium leading-relaxed";
  proc.style.color = entry.color;
  proc.textContent = entry.process;

  const content = document.createElement("span");
  content.className =
    "flex-1 min-w-0 leading-relaxed whitespace-pre overflow-hidden text-ellipsis";
  if (entry.system) {
    content.classList.add("font-medium");
    content.style.color = entry.color;
  }

  if (state.searchQuery && state.compiledRegex) {
    content.innerHTML = highlightText(entry.line);
  } else {
    content.innerHTML = entry.html;
  }

  // Click index to expand/collapse long lines
  idx.style.cursor = "pointer";
  idx.addEventListener("click", (e) => {
    e.stopPropagation();
    div.classList.toggle("expanded");
    if (div.classList.contains("expanded")) {
      content.classList.remove("overflow-hidden", "text-ellipsis", "whitespace-pre");
      content.classList.add("whitespace-pre-wrap", "break-all");
    } else {
      content.classList.add("overflow-hidden", "text-ellipsis", "whitespace-pre");
      content.classList.remove("whitespace-pre-wrap", "break-all");
    }
  });

  div.appendChild(idx);
  div.appendChild(ts);
  div.appendChild(proc);
  div.appendChild(content);

  return div;
}
