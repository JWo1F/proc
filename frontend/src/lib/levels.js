// Log level detection from plain text lines.

const LEVEL_RE = /\b(DEBUG|TRACE|INFO|WARN(?:ING)?|ERROR|ERR|FATAL|CRITICAL|CRIT|PANIC)\b/i;

const LEVEL_MAP = {
  trace: "debug",
  debug: "debug",
  info: "info",
  warn: "warn",
  warning: "warn",
  error: "error",
  err: "error",
  fatal: "fatal",
  critical: "fatal",
  crit: "fatal",
  panic: "fatal",
};

export function detectLevel(plainText) {
  const match = plainText.match(LEVEL_RE);
  if (!match) return null;
  return LEVEL_MAP[match[1].toLowerCase()] || null;
}
