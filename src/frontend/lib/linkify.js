// URL auto-linkification for HTML strings.
// Operates on already-escaped HTML, skipping inside existing <a> tags.

const URL_RE = /(?<![a-zA-Z0-9])(https?:\/\/[^\s<>"'`]+)/g;

export function linkifyHtml(html) {
  return html
    .replace(/<a\s[^>]*>[\s\S]*?<\/a>/g, "\x00$&\x00")
    .split("\x00")
    .map((part) => {
      if (part.startsWith("<a ")) return part;
      return part.replace(URL_RE, (url) => {
        const trimmed = url.replace(/[.,;:!?]+$/, "");
        const tail = url.slice(trimmed.length);
        return (
          '<a href="' +
          trimmed +
          '" target="_blank" rel="noopener noreferrer" class="ansi-link">' +
          trimmed +
          "</a>" +
          tail
        );
      });
    })
    .join("");
}
