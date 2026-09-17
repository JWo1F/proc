const ESC: u8 = b'\x1B';
const BEL: u8 = b'\x07';

/// Check if `input[pos..]` starts with an OSC 8 hyperlink sequence (`ESC]8;`).
/// Returns `Some((url, end))` where `end` is the index after the string terminator,
/// or `None` if this is not an OSC 8 sequence.
pub(crate) fn parse_osc8_at(input: &[u8], pos: usize) -> Option<(Vec<u8>, usize)> {
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

#[cfg(test)]
mod tests {
  use super::*;

  fn strip(input: &str) -> String {
    String::from_utf8(strip_ansi_except_colors(input.as_bytes())).unwrap()
  }

  #[test]
  fn plain_text_passes_through_untouched() {
    assert_eq!(strip("just a log line"), "just a log line");
    assert_eq!(strip(""), "");
  }

  #[test]
  fn color_sequences_are_kept() {
    assert_eq!(strip("\x1b[31mred\x1b[0m"), "\x1b[31mred\x1b[0m");
    assert_eq!(
      strip("\x1b[1;32mbold green\x1b[0m"),
      "\x1b[1;32mbold green\x1b[0m"
    );
    assert_eq!(
      strip("\x1b[38;5;208m256 color\x1b[0m"),
      "\x1b[38;5;208m256 color\x1b[0m"
    );
  }

  #[test]
  fn cursor_and_screen_control_is_dropped() {
    // These are what corrupt multiplexed output: one process repainting its
    // own frame would otherwise move the cursor over another's lines.
    assert_eq!(strip("\x1b[2Aup two rows"), "up two rows");
    assert_eq!(strip("\x1b[2Kerased line"), "erased line");
    assert_eq!(strip("\x1b[2Jcleared screen"), "cleared screen");
    assert_eq!(strip("\x1b[1;1Hhome"), "home");
    assert_eq!(strip("before\x1b[10Dafter"), "beforeafter");
  }

  #[test]
  fn color_survives_alongside_dropped_control() {
    assert_eq!(
      strip("\x1b[2K\x1b[33mwarning\x1b[0m\x1b[1A"),
      "\x1b[33mwarning\x1b[0m"
    );
  }

  #[test]
  fn osc8_hyperlinks_are_preserved_whole() {
    let bel = "\x1b]8;;https://example.com\x07link\x1b]8;;\x07";
    assert_eq!(strip(bel), bel);

    let st = "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\";
    assert_eq!(strip(st), st);
  }

  #[test]
  fn a_truncated_sequence_at_the_end_is_dropped() {
    assert_eq!(strip("text\x1b[31"), "text");
    assert_eq!(strip("text\x1b["), "text");
  }

  #[test]
  fn parse_osc8_at_reads_both_terminators() {
    let bel = b"\x1b]8;;https://example.com\x07";
    let (url, end) = parse_osc8_at(bel, 0).unwrap();
    assert_eq!(url, b"https://example.com");
    assert_eq!(end, bel.len());

    let st = b"\x1b]8;;https://example.com\x1b\\";
    let (url, end) = parse_osc8_at(st, 0).unwrap();
    assert_eq!(url, b"https://example.com");
    assert_eq!(end, st.len());
  }

  #[test]
  fn parse_osc8_at_rejects_anything_else() {
    assert!(parse_osc8_at(b"\x1b[31m", 0).is_none());
    assert!(parse_osc8_at(b"plain", 0).is_none());
    // Opens like OSC 8 but never terminates.
    assert!(parse_osc8_at(b"\x1b]8;;https://example.com", 0).is_none());
    // Missing the second ';' that closes the params.
    assert!(parse_osc8_at(b"\x1b]8;nope\x07", 0).is_none());
  }

  #[test]
  fn parse_osc8_at_honors_the_offset() {
    let input = b"lead\x1b]8;;https://example.com\x07";
    assert!(parse_osc8_at(input, 0).is_none());
    let (url, end) = parse_osc8_at(input, 4).unwrap();
    assert_eq!(url, b"https://example.com");
    assert_eq!(end, input.len());
  }
}
