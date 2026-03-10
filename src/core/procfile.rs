const COMMENT_PREFIX: char = '#';
const DISABLED_PREFIX: char = '_';
const NAME_CMD_SEPARATOR: &str = ":";

/// Parse Procfile lines into (name, cmd) pairs, applying exclude/include filters.
/// Lines starting with `#` are comments; names starting with `_` are disabled by default.
pub fn parse<'a>(
  input: &'a str,
  exclude: &[String],
  include: &[String],
) -> Result<Vec<(&'a str, &'a str)>, String> {
  let mut result = Vec::new();
  let mut errors = Vec::new();

  for (n, line) in input.lines().enumerate() {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with(COMMENT_PREFIX) {
      continue;
    }

    let n = n + 1;

    match line.split_once(NAME_CMD_SEPARATOR) {
      Some((name, cmd)) => {
        let name = name.trim();
        let cmd = cmd.trim();

        if name.is_empty() {
          errors.push(format!("Line {} doesn't have a name:\n> {}", n, line));
          continue;
        }

        if cmd.is_empty() {
          errors.push(format!("Line {} doesn't have a command:\n> {}", n, line));
          continue;
        }

        let (name, disabled) = match name.strip_prefix(DISABLED_PREFIX) {
          Some(stripped) => (stripped, true),
          None => (name, false),
        };

        if disabled && !include.iter().any(|i| i == name) {
          continue;
        }

        if exclude.iter().any(|e| e == name) {
          continue;
        }

        result.push((name, cmd));
      }
      None => {
        errors.push(format!(
          "Line {} should contain name and command:\n> {}",
          n, line
        ));
      }
    }
  }

  if errors.is_empty() {
    Ok(result)
  } else {
    Err(errors.join("\n"))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_accepts_indented_comments() {
    let exclude = Vec::<String>::new();
    let include = Vec::<String>::new();
    let input = "  # comment\nweb: echo hi\n";
    let parsed = parse(input, &exclude, &include).unwrap();
    assert_eq!(parsed, vec![("web", "echo hi")]);
  }
}
