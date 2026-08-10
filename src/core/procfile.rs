const COMMENT_PREFIX: char = '#';
const NAME_CMD_SEPARATOR: &str = ":";

/// Parse Procfile lines into (name, cmd) pairs.
/// If `names` is non-empty, only processes matching those names are returned.
/// Lines starting with `#` are comments.
pub fn parse<'a>(input: &'a str, names: &[String]) -> Result<Vec<(&'a str, &'a str)>, String> {
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

        if !names.is_empty() && !names.iter().any(|f| f == name) {
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
    let input = "  # comment\nweb: echo hi\n";
    let parsed = parse(input, &[]).unwrap();
    assert_eq!(parsed, vec![("web", "echo hi")]);
  }

  #[test]
  fn parse_filters_by_names() {
    let input = "web: echo web\napi: echo api\nworker: echo worker\n";
    let names = vec!["web".to_string(), "worker".to_string()];
    let parsed = parse(input, &names).unwrap();
    assert_eq!(parsed, vec![("web", "echo web"), ("worker", "echo worker")]);
  }
}
