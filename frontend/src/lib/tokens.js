// Token detection and HTML markup for cross-process log correlation.
// Detects UUIDs, IPv6, IPv4, key=value identifiers, and hex strings,
// wrapping them as clickable spans for filtering.
//
// Patterns are applied in priority order; higher-priority matches
// prevent lower-priority ones from overlapping the same text.

function escapeAttr(s) {
  return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;");
}

// Token patterns in priority order.  Each entry:
//   re    — regex (must use /g or /gi flag)
//   token — function(match) → correlation value for data-token
const PATTERNS = [
  // 1. UUID
  {
    re: /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi,
    token: (m) => m[0].toLowerCase(),
  },
  // 2. IPv6 — comprehensive pattern with lookbehind/lookahead to avoid
  //    false positives on Rust/C++ paths (std::io, etc.)
  {
    re: /(?<![:\w])(?:(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}|(?:[0-9a-f]{1,4}:){1,7}:|::(?:[0-9a-f]{1,4}:){0,6}[0-9a-f]{1,4}|(?:[0-9a-f]{1,4}:){1,6}:[0-9a-f]{1,4}|(?:[0-9a-f]{1,4}:){1,5}(?::[0-9a-f]{1,4}){1,2}|(?:[0-9a-f]{1,4}:){1,4}(?::[0-9a-f]{1,4}){1,3}|(?:[0-9a-f]{1,4}:){1,3}(?::[0-9a-f]{1,4}){1,4}|(?:[0-9a-f]{1,4}:){1,2}(?::[0-9a-f]{1,4}){1,5}|[0-9a-f]{1,4}:(?::[0-9a-f]{1,4}){1,6})(?![:\w])/gi,
    token: (m) => m[0].toLowerCase(),
  },
  // 3. IPv4
  {
    re: /\b(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})\b/g,
    token: (m) => m[0],
  },
  // 4. key=value — whole match is clickable, filter by value part
  {
    re: /\b(\w+)=([\w-]{4,})\b/g,
    token: (m) => m[2],
  },
  // 5. Standalone hex string (8–32 chars)
  {
    re: /\b[0-9a-f]{8,32}\b/gi,
    token: (m) => m[0].toLowerCase(),
  },
];

// Find all tokens in a plain text segment, resolving overlaps by priority.
function tokenifyText(text) {
  const matches = [];
  const occupied = [];

  for (const pat of PATTERNS) {
    pat.re.lastIndex = 0;
    let m;
    while ((m = pat.re.exec(text)) !== null) {
      const start = m.index;
      const end = start + m[0].length;
      let dominated = false;
      for (let i = 0; i < occupied.length; i++) {
        if (start < occupied[i][1] && end > occupied[i][0]) { dominated = true; break; }
      }
      if (dominated) continue;
      matches.push({ start, end, display: m[0], token: pat.token(m) });
      occupied.push([start, end]);
    }
  }

  if (matches.length === 0) return text;

  matches.sort((a, b) => a.start - b.start);

  let result = "";
  let pos = 0;
  for (const m of matches) {
    result += text.slice(pos, m.start);
    result += '<span class="log-token" data-token="' + escapeAttr(m.token) + '">' + m.display + "</span>";
    pos = m.end;
  }
  result += text.slice(pos);
  return result;
}

// Process HTML string, tokenifying only text nodes (outside tags).
export function tokenifyHtml(html) {
  return html.replace(/(<[^>]+>)|([^<]+)/g, (match, tag, text) => {
    if (tag) return tag;
    return tokenifyText(text);
  });
}
