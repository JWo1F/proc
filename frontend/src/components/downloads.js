import { state } from "../lib/state.js";
import { downloadAll, downloadFiltered } from "../lib/dom.js";
import { matchesFilter } from "../lib/filters.js";
import { closeAllDropdowns } from "./process-filter.js";

function downloadText(filename, text) {
  const blob = new Blob([text], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

function logsToText(entries) {
  return entries.map((e) => `${e.timestamp} [${e.process}] ${e.line}`).join("\n");
}

function dateSuffix() {
  const d = new Date();
  return (
    d.getFullYear().toString() +
    String(d.getMonth() + 1).padStart(2, "0") +
    String(d.getDate()).padStart(2, "0") +
    "-" +
    String(d.getHours()).padStart(2, "0") +
    String(d.getMinutes()).padStart(2, "0") +
    String(d.getSeconds()).padStart(2, "0")
  );
}

export function initDownloads() {
  downloadAll.addEventListener("click", () => {
    closeAllDropdowns();
    downloadText(`${state.projectName}-logs-${dateSuffix()}.txt`, logsToText(state.allLogs));
  });

  downloadFiltered.addEventListener("click", () => {
    closeAllDropdowns();
    const filtered = state.allLogs.filter(matchesFilter);
    downloadText(
      `${state.projectName}-logs-filtered-${dateSuffix()}.txt`,
      logsToText(filtered),
    );
  });
}
