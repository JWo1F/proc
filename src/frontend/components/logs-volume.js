import { on } from "../main.js";
import { formatTimeShort, formatTimestamp } from "../lib/format.js";

const LEVEL_COLORS_LIGHT = {
  fatal: "#dc2626",
  error: "#ef4444",
  warn:  "#eab308",
  info:  "#3b82f6",
  none:  "#94a3af",
  debug: "#c0c5cc",
};

const LEVEL_COLORS_DARK = {
  fatal: "#f87171",
  error: "#ef4444",
  warn:  "#facc15",
  info:  "#60a5fa",
  none:  "#6b7280",
  debug: "#4b5563",
};

// Bottom-to-top stack order
const STACK_ORDER = ["debug", "none", "info", "warn", "error", "fatal"];
const LABEL = {
  fatal: "Fatal", error: "Error", warn: "Warn",
  info: "Info", none: "No level", debug: "Debug",
};
const TOOLTIP_ORDER = ["fatal", "error", "warn", "info", "none", "debug"];

let collapsed = false;
let lastVolume = null;
let panel, chartWrap, canvas, ctx, chevron, tooltip;
let isDark = false;
let hoverIdx = -1;
let dpr = 1;
let layout = null;

export function initLogsVolume() {
  panel = document.getElementById("logs-volume-panel");
  chartWrap = document.getElementById("logs-volume-chart");
  chevron = document.getElementById("logs-volume-chevron");

  const barsEl = document.getElementById("logs-volume-bars");
  canvas = document.createElement("canvas");
  canvas.style.cssText = "width:100%;height:100%;display:block;cursor:crosshair";
  barsEl.replaceWith(canvas);
  ctx = canvas.getContext("2d");

  tooltip = document.createElement("div");
  tooltip.className = "logs-volume-tooltip hidden";
  panel.appendChild(tooltip);

  document.getElementById("logs-volume-header").addEventListener("click", () => {
    collapsed = !collapsed;
    chartWrap.classList.toggle("hidden", collapsed);
    chevron.classList.toggle("rotate-90", !collapsed);
    if (!collapsed && lastVolume) renderVolume(lastVolume);
  });

  canvas.addEventListener("mousemove", onMouseMove);
  canvas.addEventListener("mouseleave", onMouseLeave);

  const observer = new MutationObserver(() => {
    isDark = document.documentElement.classList.contains("dark");
    if (lastVolume && !collapsed) renderVolume(lastVolume);
  });
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
  isDark = document.documentElement.classList.contains("dark");

  const ro = new ResizeObserver(() => {
    if (lastVolume && !collapsed) renderVolume(lastVolume);
  });
  ro.observe(chartWrap);

  on("update", (msg) => {
    if (!msg.volume) return;
    lastVolume = msg.volume;
    panel.classList.remove("hidden");
    if (!collapsed) renderVolume(msg.volume);
  });
}

function renderVolume(vol) {
  const { buckets, start, interval } = vol;
  if (!buckets || buckets.length === 0) return;

  dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  const w = rect.width;
  const h = rect.height;
  if (w === 0 || h === 0) return;

  canvas.width = w * dpr;
  canvas.height = h * dpr;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

  const colors = isDark ? LEVEL_COLORS_DARK : LEVEL_COLORS_LIGHT;
  const n = buckets.length;

  // Compute totals
  const totals = [];
  let maxTotal = 0;
  for (let i = 0; i < n; i++) {
    let t = 0;
    for (const lvl of STACK_ORDER) t += buckets[i][lvl];
    totals.push(t);
    if (t > maxTotal) maxTotal = t;
  }
  if (maxTotal === 0) { ctx.clearRect(0, 0, w, h); return; }

  const padB = 16;
  const chartH = h - padB;

  // Bar sizing: gap scales with bucket count
  const maxBarW = 8;
  const rawBarW = w / n;
  const gap = n > 80 ? 1 : 2;
  const barW = Math.min(maxBarW, Math.max(1, rawBarW - gap));
  const step = w / n;

  layout = { n, step, barW, gap, padB, chartH, w, h, start, interval, buckets, totals, maxTotal };

  ctx.clearRect(0, 0, w, h);

  // Hover indicator (draw first, behind bars)
  if (hoverIdx >= 0 && hoverIdx < n) {
    const cx = Math.round(hoverIdx * step + step / 2);
    ctx.fillStyle = isDark ? "rgba(255,255,255,0.07)" : "rgba(0,0,0,0.05)";
    ctx.fillRect(Math.round(hoverIdx * step), 0, Math.ceil(step), chartH);
  }

  // Draw bars
  const baseY = chartH;
  for (let i = 0; i < n; i++) {
    const total = totals[i];
    if (total === 0) continue;

    const barH = Math.max(1, Math.round((total / maxTotal) * chartH));
    const x = Math.round(i * step + (step - barW) / 2);
    const barTop = baseY - barH;
    const isHovered = i === hoverIdx;

    // Collect visible segments with pixel-rounded heights (top-down)
    const segs = [];
    for (let s = STACK_ORDER.length - 1; s >= 0; s--) {
      const count = buckets[i][STACK_ORDER[s]];
      if (count === 0) continue;
      segs.push({ lvl: STACK_ORDER[s], frac: count / total });
    }

    // Distribute pixel heights top-down, last segment gets remainder to guarantee bottom alignment
    let usedH = 0;
    for (let s = 0; s < segs.length; s++) {
      const segH = s === segs.length - 1
        ? barH - usedH
        : Math.max(1, Math.round(segs[s].frac * barH));
      segs[s].y = barTop + usedH;
      segs[s].h = segH;
      usedH += segH;
    }

    ctx.globalAlpha = isHovered ? 1 : 0.8;
    for (let s = 0; s < segs.length; s++) {
      ctx.fillStyle = colors[segs[s].lvl];
      if (barW >= 3 && s === 0) {
        roundRect(ctx, x, segs[s].y, barW, segs[s].h, 1.5);
      } else {
        ctx.fillRect(x, segs[s].y, barW, segs[s].h);
      }
    }
  }

  ctx.globalAlpha = 1;

  drawTimeAxis(start, interval, n, w, h, padB);
}

function roundRect(ctx, x, y, w, h, r) {
  if (r <= 0 || h < r * 2) {
    ctx.fillRect(x, y, w, h);
    return;
  }
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.lineTo(x + w - r, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + r);
  ctx.lineTo(x + w, y + h);
  ctx.lineTo(x, y + h);
  ctx.lineTo(x, y + r);
  ctx.quadraticCurveTo(x, y, x + r, y);
  ctx.closePath();
  ctx.fill();
}

function drawTimeAxis(start, interval, n, w, h, padB) {
  const totalSpan = interval * n;
  const y = h - 1;

  const mono = getComputedStyle(document.documentElement).getPropertyValue("--font-mono").trim() || "monospace";
  ctx.font = `10px ${mono}`;
  ctx.textBaseline = "bottom";

  // Measure a representative label to compute how many ticks fit
  const sampleLabel = formatTimeShort(start, totalSpan);
  const labelW = ctx.measureText(sampleLabel).width + 16; // 16px min gap
  const targetTicks = Math.max(2, Math.min(10, Math.floor(w / labelW)));
  const niceInterval = niceTimeStep(totalSpan / targetTicks);
  const firstTick = Math.ceil(start / niceInterval) * niceInterval;

  const minGap = 8; // minimum pixels between labels
  let lastRight = -Infinity;

  for (let t = firstTick; t <= start + totalSpan; t += niceInterval) {
    const frac = (t - start) / totalSpan;
    const x = frac * w;
    if (x < 25 || x > w - 25) continue;

    const label = formatTimeShort(t, totalSpan);
    const tw = ctx.measureText(label).width;
    const left = x - tw / 2;

    // Skip if this label would overlap the previous one
    if (left < lastRight + minGap) continue;

    ctx.fillStyle = isDark ? "#374151" : "#d1d5db";
    ctx.fillRect(x, h - padB + 1, 1, 4);

    ctx.fillStyle = isDark ? "#6b7280" : "#9ca3af";
    ctx.fillText(label, left, y);
    lastRight = left + tw;
  }
}

function niceTimeStep(raw) {
  const steps = [
    0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30,     // sub-minute
    60, 120, 300, 600, 900, 1800, 3600,         // minutes/hours
    7200, 14400, 21600, 43200,                   // 2h, 4h, 6h, 12h
    86400, 172800, 604800,                       // 1d, 2d, 7d
  ];
  for (const s of steps) {
    if (s >= raw * 0.7) return s;
  }
  return 604800;
}

function formatTime(unixSec, ms) {
  return formatTimestamp(unixSec, ms);
}

function onMouseMove(e) {
  if (!layout) return;
  const rect = canvas.getBoundingClientRect();
  const x = e.clientX - rect.left;
  const idx = Math.floor(x / layout.step);

  if (idx < 0 || idx >= layout.n) { onMouseLeave(); return; }

  if (idx !== hoverIdx) {
    hoverIdx = idx;
    renderVolume(lastVolume);
  }
  showTooltip(e, layout.buckets[idx], layout.start, layout.interval, idx);
}

function onMouseLeave() {
  if (hoverIdx !== -1) {
    hoverIdx = -1;
    if (lastVolume) renderVolume(lastVolume);
  }
  tooltip.classList.add("hidden");
}

function showTooltip(e, bucket, start, interval, idx) {
  const from = start + idx * interval;
  const to = from + interval;
  const ms = interval < 1;

  let total = 0;
  for (const lvl of TOOLTIP_ORDER) total += bucket[lvl];

  const colors = isDark ? LEVEL_COLORS_DARK : LEVEL_COLORS_LIGHT;

  let html = `<div class="logs-volume-tooltip-time">${formatTime(from, ms)} – ${formatTime(to, ms)}</div>`;
  for (const lvl of TOOLTIP_ORDER) {
    if (bucket[lvl] === 0) continue;
    html += `<div class="logs-volume-tooltip-row">
      <span class="logs-volume-tooltip-dot" style="background:${colors[lvl]}"></span>
      <span>${LABEL[lvl]}</span>
      <span class="logs-volume-tooltip-count">${bucket[lvl].toLocaleString()}</span>
    </div>`;
  }
  html += `<div class="logs-volume-tooltip-total">Total: ${total.toLocaleString()}</div>`;

  tooltip.innerHTML = html;
  tooltip.classList.remove("hidden");

  const panelRect = panel.getBoundingClientRect();
  let tx = e.clientX - panelRect.left + 12;
  const ty = e.clientY - panelRect.top - 10;
  const tw = tooltip.offsetWidth;
  if (tx + tw > panelRect.width) tx = tx - tw - 24;
  tooltip.style.left = tx + "px";
  tooltip.style.top = ty + "px";
}
