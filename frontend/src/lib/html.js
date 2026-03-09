// HTML escaping, URL linkification, and search highlighting.

import { state } from "./state.js";

export function escapeHtml(str) {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

const URL_RE = /(?<![a-zA-Z0-9])(https?:\/\/[^\s<>"'`]+)/g;

// Auto-link bare URLs in already-escaped HTML, avoiding double-linking inside <a> tags
export function linkifyHtml(html) {
  return html
    .replace(/<a\s[^>]*>[\s\S]*?<\/a>/g, "\x00$&\x00")
    .split("\x00")
    .map((part) => {
      if (part.startsWith("<a ")) return part;
      return part.replace(URL_RE, (url) => {
        let trimmed = url.replace(/[.,;:!?]+$/, "");
        let tail = url.slice(trimmed.length);
        return (
          '<a href="' + trimmed + '" target="_blank" rel="noopener noreferrer" class="ansi-link">' +
          trimmed + "</a>" + tail
        );
      });
    })
    .join("");
}

// Apply search highlights to an escaped HTML string, skipping inside tags
export function applyHighlights(html) {
  if (!state.compiledRegex) return html;
  return html.replace(/(<[^>]+>)|([^<]+)/g, (m, tag, text) => {
    if (tag) return tag;
    state.compiledRegex.lastIndex = 0;
    return text.replace(
      state.compiledRegex,
      '<span class="search-highlight">$&</span>',
    );
  });
}

// Build display HTML: linkify first, then highlight search matches
export function highlightText(text) {
  return applyHighlights(linkifyHtml(escapeHtml(text)));
}
