// Shared HTML escaping utilities.

export function escapeHtml(ch) {
  if (ch === "&") return "&amp;";
  if (ch === "<") return "&lt;";
  if (ch === ">") return "&gt;";
  if (ch === '"') return "&quot;";
  return ch;
}

export function escapeAttr(s) {
  return s.replace(/[&"<>]/g, escapeHtml);
}
