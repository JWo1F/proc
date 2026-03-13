// Shared formatting utilities.

export const pad2 = (n) => (n < 10 ? "0" : "") + n;
export const pad3 = (n) => (n < 10 ? "00" : n < 100 ? "0" : "") + n;

export function formatTimestamp(unixSeconds, ms) {
  const d = new Date(unixSeconds * 1000);
  const base =
    pad2(d.getHours()) + ":" + pad2(d.getMinutes()) + ":" + pad2(d.getSeconds());
  if (ms) return base + "." + pad3(d.getMilliseconds());
  return base;
}

export function formatTimeShort(unixSec, totalSpan) {
  const d = new Date(unixSec * 1000);
  if (totalSpan < 60) return pad2(d.getMinutes()) + ":" + pad2(d.getSeconds());
  return pad2(d.getHours()) + ":" + pad2(d.getMinutes()) + ":" + pad2(d.getSeconds());
}

export function fmtSize(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + " MB";
  return (bytes / (1024 * 1024 * 1024)).toFixed(2) + " GB";
}
