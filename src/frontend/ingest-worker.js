// Ingest Worker — owns the SSE connection, parses log lines,
// buffers them, and sends batches to the DB worker via MessagePort.

import { stripAnsi } from "./lib/ansi.js";
import { detectLevel } from "./lib/levels.js";

// ── MessagePort to DB worker (received via "connect" message) ────────

let dbPort = null;

// ── Stats ────────────────────────────────────────────────────────────

let received = 0;
let flushed = 0;
let statsTimer = null;

function reportStats() {
  self.postMessage({
    type: "ingestStats",
    received,
    flushed,
    queued: writeBuf.length,
  });
}

function scheduleStats() {
  if (statsTimer) return;
  statsTimer = setTimeout(() => {
    statsTimer = null;
    reportStats();
  }, 200);
}

// ── Write buffer ─────────────────────────────────────────────────────

let writeBuf = [];
let writeTimer = null;

function flushWrites() {
  writeTimer = null;
  if (writeBuf.length === 0) return;
  const batch = writeBuf;
  writeBuf = [];
  flushed += batch.length;
  dbPort.postMessage({ type: "putBatch", entries: batch });
  scheduleStats();
}

function scheduleWrite(entry) {
  writeBuf.push(entry);
  if (writeBuf.length >= 1000) {
    if (writeTimer) { clearTimeout(writeTimer); writeTimer = null; }
    flushWrites();
  } else if (!writeTimer) {
    writeTimer = setTimeout(flushWrites, 50);
  }
}

// ── Timestamp parsing ────────────────────────────────────────────────

const ISO_TS_RE =
  /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))\s*/;

function parseLineTimestamp(plain) {
  const m = ISO_TS_RE.exec(plain);
  if (!m) return null;
  const d = new Date(m[1]);
  if (isNaN(d.getTime())) return null;
  return { ts: d.getTime() / 1000, prefixLen: m[0].length };
}

// ── SSE connection ──────────────────────────────────────────────────

let eventSource = null;

function connect() {
  if (eventSource) eventSource.close();
  writeBuf.length = 0;
  if (writeTimer) { clearTimeout(writeTimer); writeTimer = null; }

  eventSource = new EventSource("/api/sse");

  eventSource.addEventListener("init", (e) => {
    try {
      const info = JSON.parse(e.data);
      self.postMessage({ type: "init", name: info.name || "procfile" });
    } catch {}
  });

  eventSource.addEventListener("message", (e) => {
    try {
      const raw = JSON.parse(e.data);
      const index = parseInt(e.lastEventId, 10);
      let plain = stripAnsi(raw.line);
      const level = detectLevel(plain);

      let ts = raw.ts;
      let hasMs = false;
      let rawLine = raw.line;
      const parsed = parseLineTimestamp(plain);
      if (parsed) {
        ts = parsed.ts;
        hasMs = true;
        plain = plain.slice(parsed.prefixLen);
        let ri = 0;
        let pi = 0;
        while (ri < rawLine.length && pi < parsed.prefixLen) {
          if (rawLine[ri] === "\x1b") {
            const end = rawLine.indexOf("m", ri);
            ri = end >= 0 ? end + 1 : ri + 1;
          } else {
            pi++;
            ri++;
          }
        }
        rawLine = rawLine.slice(ri);
      }

      received++;
      scheduleStats();

      scheduleWrite({
        index,
        process: raw.process,
        color: raw.color,
        raw: rawLine,
        plain,
        level,
        system: raw.sys,
        ts,
        hasMs,
      });
    } catch (err) {
      console.error("Failed to parse SSE event:", err);
    }
  });

  eventSource.addEventListener("open", () => {
    self.postMessage({ type: "connected", value: true });
  });

  eventSource.addEventListener("error", () => {
    self.postMessage({ type: "connected", value: false });
    eventSource.close();
  });
}

// ── Message handler ─────────────────────────────────────────────────

self.onmessage = (e) => {
  const msg = e.data;
  switch (msg.type) {
    case "connect":
      dbPort = msg.dbPort;
      connect();
      break;
  }
};
