const ESC: u8 = b'\x1B';

/// Strip ANSI escape sequences from PTY output, keeping only color codes (SGR, ending with 'm').
/// This removes cursor movement, screen clearing, and other terminal control sequences
/// that would corrupt the multiplexed output while preserving colored text.
pub fn strip_ansi_except_colors(input: &[u8]) -> Vec<u8> {
  let mut result = Vec::with_capacity(input.len());
  let mut chars = input.iter().copied().enumerate();

  while let Some((i, byte)) = chars.next() {
    if byte != ESC || input.get(i + 1) != Some(&b'[') {
      result.push(byte);
      continue;
    }

    // Skip '[' after ESC
    chars.next();

    // Collect the sequence: digits and ';' until a final letter
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
  }

  result
}
