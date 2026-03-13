// ANSI escape code parser — converts terminal color codes to HTML spans.

import { escapeHtml, escapeAttr } from "./html.js";

const STANDARD = [
  "#4e4e4e",
  "#cd3131",
  "#0dbc79",
  "#e5e510",
  "#2472c8",
  "#bc3fbc",
  "#11a8cd",
  "#e5e5e5",
];
const BRIGHT = [
  "#666666",
  "#f14c4c",
  "#23d18b",
  "#f5f543",
  "#3b8eea",
  "#d670d6",
  "#29b8db",
  "#ffffff",
];

function ansi256(n) {
  if (n < 8) return STANDARD[n];
  if (n < 16) return BRIGHT[n - 8];
  if (n < 232) {
    const i = n - 16;
    return `rgb(${((i / 36) | 0) * 51},${(((i % 36) / 6) | 0) * 51},${(i % 6) * 51})`;
  }
  const g = 8 + (n - 232) * 10;
  return `rgb(${g},${g},${g})`;
}

function sgrToColor(params) {
  const p = params.split(";").map(Number);
  if (p[0] === 38 && p[1] === 5 && p.length >= 3) return ansi256(p[2]);
  if (p[0] === 38 && p[1] === 2 && p.length >= 5)
    return `rgb(${p[2]},${p[3]},${p[4]})`;
  if (p.length === 1 && p[0] >= 30 && p[0] <= 37) return STANDARD[p[0] - 30];
  if (p.length === 1 && p[0] >= 90 && p[0] <= 97) return BRIGHT[p[0] - 90];
  return null;
}

// Parse OSC 8 hyperlink at position i. Returns { url, end } or null.
function parseOsc8(input, i) {
  if (input.substr(i, 4) !== "\x1b]8;") return null;
  const paramsEnd = input.indexOf(";", i + 4);
  if (paramsEnd === -1) return null;
  const urlStart = paramsEnd + 1;
  // Find string terminator: BEL (\x07) or ST (\x1b\\)
  let j = urlStart;
  while (j < input.length) {
    if (input[j] === "\x07")
      return { url: input.slice(urlStart, j), end: j + 1 };
    if (input[j] === "\x1b" && input[j + 1] === "\\")
      return { url: input.slice(urlStart, j), end: j + 2 };
    j++;
  }
  return null;
}

export function ansiToHtml(input) {
  let result = "";
  let spanOpen = false;
  let linkOpen = false;
  let i = 0;

  while (i < input.length) {
    if (input[i] === "\x1b") {
      // OSC 8 hyperlink
      const osc = parseOsc8(input, i);
      if (osc) {
        i = osc.end;
        if (!osc.url) {
          if (linkOpen) {
            result += "</a>";
            linkOpen = false;
          }
        } else if (/^https?:\/\/|^mailto:/.test(osc.url)) {
          if (linkOpen) result += "</a>";
          result += `<a href="${escapeAttr(osc.url)}" target="_blank" rel="noopener noreferrer" class="ansi-link">`;
          linkOpen = true;
        }
        continue;
      }

      // CSI sequence (ESC [)
      if (input[i + 1] === "[") {
        i += 2;
        let params = "";
        while (
          i < input.length &&
          ((input[i] >= "0" && input[i] <= "9") || input[i] === ";")
        ) {
          params += input[i++];
        }
        if (i < input.length && input[i] === "m") {
          i++;
          const color = sgrToColor(params);
          if (color) {
            if (spanOpen) result += "</span>";
            result += `<span style="color:${color}">`;
            spanOpen = true;
          } else if (params === "0" || params === "") {
            if (spanOpen) {
              result += "</span>";
              spanOpen = false;
            }
          }
        } else if (i < input.length) {
          i++; // skip unknown final byte
        }
        continue;
      }

      // Other ESC sequence — skip ESC and next byte
      i += 2;
      continue;
    }

    result += escapeHtml(input[i]);
    i++;
  }

  if (spanOpen) result += "</span>";
  if (linkOpen) result += "</a>";
  return result;
}

export function stripAnsi(input) {
  let result = "";
  let i = 0;
  while (i < input.length) {
    if (input[i] === "\x1b") {
      const osc = parseOsc8(input, i);
      if (osc) {
        i = osc.end;
        continue;
      }
      if (input[i + 1] === "[") {
        i += 2;
        while (
          i < input.length &&
          ((input[i] >= "0" && input[i] <= "9") || input[i] === ";")
        )
          i++;
        if (i < input.length) i++; // skip final byte
        continue;
      }
      i += 2;
      continue;
    }
    result += input[i];
    i++;
  }
  return result;
}
