pub fn strip_ansi_except_colors(input: &[u8]) -> Vec<u8> {
  let mut result = Vec::with_capacity(input.len());
  let mut i = 0;

  while i < input.len() {
    if i + 1 < input.len() && input[i] == b'\x1B' && input[i + 1] == b'[' {
      // Found an ANSI escape sequence
      let mut end = i + 2;
      while end < input.len() && (input[end].is_ascii_digit() || input[end] == b';') {
        end += 1;
      }
      if end < input.len() && input[end] == b'm' {
        // Color sequence (ends with 'm') — keep it
        result.extend_from_slice(&input[i..=end]);
        i = end + 1;
      } else {
        // Non-color ANSI sequence — skip it
        i = end + 1;
      }
    } else {
      result.push(input[i]);
      i += 1;
    }
  }

  result.shrink_to_fit();
  result
}
