import { ui } from "../main.js";
import { downloadAll, downloadFiltered } from "../lib/dom.js";
import { closeAllDropdowns } from "./process-filter.js";

let requestId = 0;
const pending = new Map();

function onWorkerResponse(e) {
  const msg = e.data;
  if (msg.type !== "allLogs" && msg.type !== "filteredLogs") return;
  const cb = pending.get(msg.id);
  if (cb) {
    pending.delete(msg.id);
    cb(msg.entries);
  }
}

function requestLogs(type) {
  return new Promise((resolve) => {
    const id = ++requestId;
    pending.set(id, resolve);
    ui.worker.postMessage({ type, id });
  });
}

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
  ui.worker.addEventListener("message", onWorkerResponse);

  downloadAll.addEventListener("click", async () => {
    closeAllDropdowns();
    const entries = await requestLogs("getAll");
    downloadText(`${ui.projectName}-logs-${dateSuffix()}.txt`, logsToText(entries));
  });

  downloadFiltered.addEventListener("click", async () => {
    closeAllDropdowns();
    const entries = await requestLogs("getFiltered");
    downloadText(`${ui.projectName}-logs-filtered-${dateSuffix()}.txt`, logsToText(entries));
  });
}
