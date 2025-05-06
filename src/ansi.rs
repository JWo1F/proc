pub fn strip_ansi_except_colors(input: Vec<u8>) -> Vec<u8> {
  let mut result = Vec::with_capacity(input.len());
  let mut i = 0;
  let input_len = input.len();

  while i < input_len {
    if i + 1 < input_len && input[i] == b'\x1B' && input[i + 1] == b'[' {
      // Найдена ANSI-последовательность
      let mut end = i + 2;
      while end < input_len && (input[end].is_ascii_digit() || input[end] == b';') {
        end += 1;
      }
      if end < input_len && input[end] == b'm' {
        // Это цветовая последовательность (заканчивается на 'm')
        result.extend_from_slice(&input[i..=end]);
        i = end + 1;
      } else {
        // Пропускаем нецветовую ANSI-последовательность
        i = end + 1;
      }
    } else {
      // Обычный символ
      result.push(input[i]);
      i += 1;
    }
  }

  result.shrink_to_fit();

  result
}
