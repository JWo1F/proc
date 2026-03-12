import { onWorkerMessage } from "../main.js";

const LEVEL_COLORS = {
  fatal: "#dc2626",
  error: "#ef4444",
  warn: "#eab308",
  info: "#3b82f6",
  none: "#6b7280",
  debug: "#9ca3af",
};

// Top of bar to bottom
const RENDER_ORDER = ["fatal", "error", "warn", "info", "none", "debug"];
const LABEL = {
  fatal: "Fatal",
  error: "Error",
  warn: "Warn",
  info: "Info",
  none: "No level",
  debug: "Debug",
};

let collapsed = false;
let lastVolume = null;
let panel, chart, barsEl, chevron, tooltip;

export function initLogsVolume() {
  panel = document.getElementById("logs-volume-panel");
  chart = document.getElementById("logs-volume-chart");
  barsEl = document.getElementById("logs-volume-bars");
  chevron = document.getElementById("logs-volume-chevron");

  tooltip = document.createElement("div");
  tooltip.className = "logs-volume-tooltip hidden";
  panel.appendChild(tooltip);

  document.getElementById("logs-volume-header").addEventListener("click", () => {
    collapsed = !collapsed;
    chart.classList.toggle("hidden", collapsed);
    chevron.classList.toggle("rotate-90", !collapsed);
    if (!collapsed && lastVolume) renderVolume(lastVolume);
  });

  onWorkerMessage("update", (msg) => {
    if (!msg.volume) return;
    lastVolume = msg.volume;
    panel.classList.remove("hidden");
    if (!collapsed) renderVolume(msg.volume);
  });
}

function renderVolume(vol) {
  const { buckets, start, interval } = vol;
  if (!buckets || buckets.length === 0) return;

  let maxTotal = 0;
  for (const b of buckets) {
    let t = 0;
    for (const lvl of RENDER_ORDER) t += b[lvl];
    if (t > maxTotal) maxTotal = t;
  }
  if (maxTotal === 0) return;

  const frag = document.createDocumentFragment();

  for (let i = 0; i < buckets.length; i++) {
    const b = buckets[i];
    let total = 0;
    for (const lvl of RENDER_ORDER) total += b[lvl];

    const bar = document.createElement("div");
    bar.className = "logs-volume-bar";
    bar.style.height = total > 0 ? `${(total / maxTotal) * 100}%` : "0";

    for (const lvl of RENDER_ORDER) {
      if (b[lvl] === 0) continue;
      const seg = document.createElement("div");
      seg.style.height = `${(b[lvl] / total) * 100}%`;
      seg.style.backgroundColor = LEVEL_COLORS[lvl];
      seg.style.minHeight = "1px";
      bar.appendChild(seg);
    }

    bar.addEventListener("mouseenter", (e) =>
      showTooltip(e, b, total, start, interval, i),
    );
    bar.addEventListener("mousemove", positionTooltip);
    bar.addEventListener("mouseleave", hideTooltip);

    frag.appendChild(bar);
  }

  barsEl.replaceChildren(frag);
}

function formatTime(unixSec) {
  const d = new Date(unixSec * 1000);
  const p = (n) => (n < 10 ? "0" : "") + n;
  return p(d.getHours()) + ":" + p(d.getMinutes()) + ":" + p(d.getSeconds());
}

function showTooltip(e, bucket, total, start, interval, idx) {
  const from = start + idx * interval;
  const to = from + interval;

  let html = `<div class="logs-volume-tooltip-time">${formatTime(from)} – ${formatTime(to)}</div>`;
  for (const lvl of RENDER_ORDER) {
    if (bucket[lvl] === 0) continue;
    html += `<div class="logs-volume-tooltip-row">
      <span class="logs-volume-tooltip-dot" style="background:${LEVEL_COLORS[lvl]}"></span>
      <span>${LABEL[lvl]}</span>
      <span class="logs-volume-tooltip-count">${bucket[lvl]}</span>
    </div>`;
  }
  html += `<div class="logs-volume-tooltip-total">Total: ${total}</div>`;

  tooltip.innerHTML = html;
  tooltip.classList.remove("hidden");
  positionTooltip(e);
}

function positionTooltip(e) {
  const rect = panel.getBoundingClientRect();
  let x = e.clientX - rect.left + 12;
  const y = e.clientY - rect.top - 10;

  const tw = tooltip.offsetWidth;
  if (x + tw > rect.width) x = x - tw - 24;

  tooltip.style.left = x + "px";
  tooltip.style.top = y + "px";
}

function hideTooltip() {
  tooltip.classList.add("hidden");
}
