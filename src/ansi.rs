const ESC: u8 = b'\x1B';
const BEL: u8 = b'\x07';

/// Check if `input[pos..]` starts with an OSC 8 hyperlink sequence (`ESC]8;`).
/// Returns `Some((url, end))` where `end` is the index after the string terminator,
/// or `None` if this is not an OSC 8 sequence.
fn parse_osc8_at(input: &[u8], pos: usize) -> Option<(Vec<u8>, usize)> {
  // Need at least ESC ] 8 ;
  if input.get(pos) != Some(&ESC)
    || input.get(pos + 1) != Some(&b']')
    || input.get(pos + 2) != Some(&b'8')
    || input.get(pos + 3) != Some(&b';')
  {
    return None;
  }

  // Skip params (between the two ';') — OSC 8 format: ESC]8;params;uri ST
  let after_8_semi = pos + 4;
  let params_end = input[after_8_semi..].iter().position(|&b| b == b';')?;
  let uri_start = after_8_semi + params_end + 1;

  // Find the string terminator: BEL (\x07) or ST (ESC \)
  let mut i = uri_start;
  while i < input.len() {
    if input[i] == BEL {
      let url = input[uri_start..i].to_vec();
      return Some((url, i + 1));
    }
    if input[i] == ESC && input.get(i + 1) == Some(&b'\\') {
      let url = input[uri_start..i].to_vec();
      return Some((url, i + 2));
    }
    i += 1;
  }

  None
}

/// Strip ANSI escape sequences from PTY output, keeping only color codes (SGR, ending with 'm')
/// and OSC 8 hyperlink sequences. This removes cursor movement, screen clearing, and other
/// terminal control sequences that would corrupt the multiplexed output.
pub fn strip_ansi_except_colors(input: &[u8]) -> Vec<u8> {
  let mut result = Vec::with_capacity(input.len());
  let mut chars = input.iter().copied().enumerate();

  while let Some((i, byte)) = chars.next() {
    if byte != ESC {
      result.push(byte);
      continue;
    }

    // OSC 8 hyperlink — preserve entirely
    if let Some((_, end)) = parse_osc8_at(input, i) {
      result.extend_from_slice(&input[i..end]);
      // Advance the iterator past the sequence
      for _ in 0..(end - i - 1) {
        chars.next();
      }
      continue;
    }

    // CSI sequence (ESC [)
    if input.get(i + 1) == Some(&b'[') {
      // Skip '['
      chars.next();

      let seq_start = i;
      let final_byte = loop {
        match chars.next() {
          Some((_, b)) if b.is_ascii_digit() || b == b';' => continue,
          Some((end, b)) => break Some((end, b)),
          None => break None,
        }
      };

      // Keep color sequences (ending with 'm'), drop everything else
      if let Some((end, b'm')) = final_byte {
        result.extend_from_slice(&input[seq_start..=end]);
      }
      continue;
    }

    // Other ESC sequence — drop
    result.push(byte);
  }

  result
}

/// Strip all ANSI escape sequences, returning plain text.
/// OSC 8 hyperlinks are stripped but their display text is kept.
#[cfg(feature = "web")]
pub fn strip_ansi_codes(input: &str) -> String {
  let mut result = String::with_capacity(input.len());
  let bytes = input.as_bytes();
  let mut i = 0;

  while i < bytes.len() {
    if bytes[i] == ESC {
      // OSC 8 hyperlink — skip the escape sequence, text after it is kept naturally
      if let Some((_, end)) = parse_osc8_at(bytes, i) {
        i = end;
        continue;
      }
      // CSI sequence (ESC [)
      if bytes.get(i + 1) == Some(&b'[') {
        i += 2;
        while i < bytes.len() {
          let b = bytes[i];
          i += 1;
          if b.is_ascii_alphabetic() {
            break;
          }
        }
        continue;
      }
    }
    let ch_len = utf8_char_len(bytes[i]);
    result.push_str(&input[i..i + ch_len]);
    i += ch_len;
  }

  result
}

/// Return the byte length of the UTF-8 character starting with this byte.
fn utf8_char_len(b: u8) -> usize {
  match b {
    0..=0x7F => 1,
    0xC0..=0xDF => 2,
    0xE0..=0xEF => 3,
    0xF0..=0xF7 => 4,
    _ => 1, // continuation byte, shouldn't happen at start
  }
}

/// Convert a string with ANSI color codes and OSC 8 hyperlinks into HTML.
/// SGR color codes become `<span style="color:...">`, OSC 8 links become `<a href="...">`.
/// Raw URLs (http/https) in plain text are also auto-linked.
#[cfg(feature = "web")]
pub fn ansi_to_html(input: &str) -> String {
  let mut result = String::with_capacity(input.len());
  let bytes = input.as_bytes();
  let mut i = 0;
  let mut span_open = false;
  let mut link_open = false;
  // Buffer plain text so we can auto-linkify URLs when flushing
  let mut text_buf = String::new();

  while i < bytes.len() {
    if bytes[i] == ESC {
      // Flush buffered text before processing escape
      if !text_buf.is_empty() {
        flush_text_with_links(&mut result, &text_buf, link_open);
        text_buf.clear();
      }

      // OSC 8 hyperlink
      if let Some((url, end)) = parse_osc8_at(bytes, i) {
        i = end;
        if url.is_empty() {
          if link_open {
            result.push_str("</a>");
            link_open = false;
          }
        } else if let Ok(url_str) = std::str::from_utf8(&url) {
          if url_str.starts_with("http://")
            || url_str.starts_with("https://")
            || url_str.starts_with("mailto:")
          {
            if link_open {
              result.push_str("</a>");
            }
            result.push_str("<a href=\"");
            push_html_attr_escaped(&mut result, url_str);
            result.push_str(
              "\" target=\"_blank\" rel=\"noopener noreferrer\" class=\"ansi-link\">",
            );
            link_open = true;
          }
        }
        continue;
      }

      // CSI sequence (ESC [)
      if bytes.get(i + 1) == Some(&b'[') {
        i += 2;
        let param_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b';') {
          i += 1;
        }
        if i < bytes.len() && bytes[i] == b'm' {
          let params: &str = std::str::from_utf8(&bytes[param_start..i]).unwrap_or("");
          i += 1;

          if let Some(color) = sgr_to_css_color(params) {
            if span_open {
              result.push_str("</span>");
            }
            result.push_str(&format!("<span style=\"color:{}\">", color));
            span_open = true;
          } else if params == "0" || params.is_empty() {
            if span_open {
              result.push_str("</span>");
              span_open = false;
            }
          }
        } else if i < bytes.len() {
          i += 1;
        }
        continue;
      }
    }

    let ch_len = utf8_char_len(bytes[i]);
    text_buf.push_str(&input[i..i + ch_len]);
    i += ch_len;
  }

  // Flush remaining text
  if !text_buf.is_empty() {
    flush_text_with_links(&mut result, &text_buf, link_open);
  }

  if span_open {
    result.push_str("</span>");
  }
  if link_open {
    result.push_str("</a>");
  }

  result
}

/// Flush plain text to the result, auto-linking bare URLs.
/// If already inside an OSC 8 `<a>` tag, skip auto-linking.
#[cfg(feature = "web")]
fn flush_text_with_links(out: &mut String, text: &str, inside_link: bool) {
  if inside_link {
    push_html_escaped(out, text);
    return;
  }

  let mut rest = text;
  while let Some(url_start) = find_url_start(rest) {
    // Output text before the URL
    push_html_escaped(out, &rest[..url_start]);

    let after = &rest[url_start..];
    let url_len = url_extent(after);
    let url = &after[..url_len];

    out.push_str("<a href=\"");
    push_html_attr_escaped(out, url);
    out.push_str("\" target=\"_blank\" rel=\"noopener noreferrer\" class=\"ansi-link\">");
    push_html_escaped(out, url);
    out.push_str("</a>");

    rest = &after[url_len..];
  }

  // Remaining text after last URL (or all text if no URLs)
  push_html_escaped(out, rest);
}

/// Find the byte offset of the next `http://` or `https://` in `s`.
#[cfg(feature = "web")]
fn find_url_start(s: &str) -> Option<usize> {
  let bytes = s.as_bytes();
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'h' {
      if s[i..].starts_with("https://") || s[i..].starts_with("http://") {
        // Don't match if preceded by an alphanumeric char (part of a word like "xhttp://")
        if i == 0 || !bytes[i - 1].is_ascii_alphanumeric() {
          return Some(i);
        }
      }
    }
    i += 1;
  }
  None
}

/// Return how many bytes the URL spans starting from `s` (which begins with http(s)://).
#[cfg(feature = "web")]
fn url_extent(s: &str) -> usize {
  let mut len = 0;
  let mut paren_depth: i32 = 0;

  for ch in s.chars() {
    // Stop at whitespace or control characters
    if ch.is_whitespace() || ch.is_control() {
      break;
    }
    match ch {
      '(' => paren_depth += 1,
      ')' => {
        if paren_depth <= 0 {
          break;
        }
        paren_depth -= 1;
      }
      // Strip common trailing punctuation that's not part of URLs
      '>' | '<' | '"' | '\'' | '`' => break,
      _ => {}
    }
    len += ch.len_utf8();
  }

  // Trim trailing punctuation that's usually not part of the URL
  let trimmed = s[..len].trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?'));
  trimmed.len()
}

#[cfg(feature = "web")]
fn push_html_escaped(out: &mut String, s: &str) {
  for ch in s.chars() {
    match ch {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      '"' => out.push_str("&quot;"),
      _ => out.push(ch),
    }
  }
}

/// Escape a string for use inside an HTML attribute value.
#[cfg(feature = "web")]
fn push_html_attr_escaped(out: &mut String, s: &str) {
  for ch in s.chars() {
    match ch {
      '&' => out.push_str("&amp;"),
      '"' => out.push_str("&quot;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      _ => out.push(ch),
    }
  }
}

/// Parse SGR parameters and return a CSS color string if it's a foreground color.
#[cfg(feature = "web")]
fn sgr_to_css_color(params: &str) -> Option<String> {
  let parts: Vec<u32> = params.split(';').filter_map(|s| s.parse().ok()).collect();

  match parts.as_slice() {
    // 256-color: ESC[38;5;Nm
    [38, 5, n] => Some(ansi256_to_css(*n)),
    // True color: ESC[38;2;R;G;Bm
    [38, 2, r, g, b] => Some(format!("rgb({},{},{})", r, g, b)),
    // Standard foreground colors 30-37
    [n] if (30..=37).contains(n) => Some(standard_color(*n - 30)),
    // Bright foreground colors 90-97
    [n] if (90..=97).contains(n) => Some(bright_color(*n - 90)),
    _ => None,
  }
}

#[cfg(feature = "web")]
fn standard_color(index: u32) -> String {
  match index {
    0 => "#4e4e4e".to_string(), // black (lighter for visibility)
    1 => "#cd3131".to_string(), // red
    2 => "#0dbc79".to_string(), // green
    3 => "#e5e510".to_string(), // yellow
    4 => "#2472c8".to_string(), // blue
    5 => "#bc3fbc".to_string(), // magenta
    6 => "#11a8cd".to_string(), // cyan
    7 => "#e5e5e5".to_string(), // white
    _ => "inherit".to_string(),
  }
}

#[cfg(feature = "web")]
fn bright_color(index: u32) -> String {
  match index {
    0 => "#666666".to_string(),
    1 => "#f14c4c".to_string(),
    2 => "#23d18b".to_string(),
    3 => "#f5f543".to_string(),
    4 => "#3b8eea".to_string(),
    5 => "#d670d6".to_string(),
    6 => "#29b8db".to_string(),
    7 => "#ffffff".to_string(),
    _ => "inherit".to_string(),
  }
}

#[cfg(feature = "web")]
fn ansi256_to_css(n: u32) -> String {
  if n < 8 {
    return standard_color(n);
  }
  if n < 16 {
    return bright_color(n - 8);
  }
  if n < 232 {
    // 6x6x6 color cube
    let idx = n - 16;
    let r = (idx / 36) * 51;
    let g = ((idx % 36) / 6) * 51;
    let b = (idx % 6) * 51;
    return format!("rgb({},{},{})", r, g, b);
  }
  // Grayscale ramp
  let gray = 8 + (n - 232) * 10;
  format!("rgb({},{},{})", gray, gray, gray)
}
