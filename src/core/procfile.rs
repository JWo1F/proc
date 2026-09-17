use super::manager::OnExit;
use std::time::Duration;

const COMMENT_PREFIX: char = '#';
const NAME_CMD_SEPARATOR: &str = ":";
const FLAGS_OPEN: char = '(';
const FLAGS_CLOSE: char = ')';

const OPTIONAL: &str = "optional";
const ONCE: &str = "once";
const RESTART: &str = "restart";
const STOP: &str = "stop";
const MUTED: &str = "muted";
const ALLOW_FAILURE: &str = "allow-failure";
const DELAY: &str = "delay";
const RETRIES: &str = "retries";

const KNOWN_FLAGS: &str =
  "optional, once, restart, stop, muted, allow-failure, delay=<duration>, retries=<n>";

/// Per-process options declared in parentheses after the name:
/// `worker(once, optional): rake jobs:work`.
///
/// Flags are scoped to the one process that carries them — they never change
/// how anything else in the Procfile runs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flags {
  /// Declared `optional`: kept out of the run until someone asks for it by
  /// name, with `--enable`, or from the interactive menu.
  pub optional: bool,
  /// Overrides the session-wide `--on-exit` policy for this process alone.
  /// `None` means it follows whatever the session is set to.
  pub on_exit: Option<OnExit>,
  /// Declared `muted`: its output stays out of the terminal from the start.
  pub muted: bool,
  /// Declared `allow-failure`: a non-zero exit from this process does not make
  /// the run as a whole fail.
  pub allow_failure: bool,
  /// How long to hold the automatic start at the beginning of the session.
  /// An explicit start is always immediate.
  pub delay: Option<Duration>,
  /// Ceiling on consecutive automatic restarts before giving up on it.
  pub retries: Option<u32>,
}

impl Flags {
  /// The flag words that would reproduce this set, in declaration order.
  pub fn labels(&self) -> Vec<String> {
    let mut labels = Vec::new();
    if self.optional {
      labels.push(OPTIONAL.to_string());
    }
    match self.on_exit {
      Some(OnExit::Ignore) => labels.push(ONCE.to_string()),
      Some(OnExit::Restart) => labels.push(RESTART.to_string()),
      Some(OnExit::Stop) => labels.push(STOP.to_string()),
      None => {}
    }
    if let Some(delay) = self.delay {
      labels.push(format!("{}={}", DELAY, format_duration(delay)));
    }
    if let Some(retries) = self.retries {
      labels.push(format!("{}={}", RETRIES, retries));
    }
    if self.muted {
      labels.push(MUTED.to_string());
    }
    if self.allow_failure {
      labels.push(ALLOW_FAILURE.to_string());
    }
    labels
  }

  /// `"(optional, once)"`, or an empty string when nothing is set — so it can
  /// be appended straight onto a name.
  pub fn suffix(&self) -> String {
    let labels = self.labels();
    if labels.is_empty() {
      String::new()
    } else {
      format!("({})", labels.join(", "))
    }
  }
}

/// Parse `500ms`, `2s`, `1m`, or a bare number of seconds.
fn parse_duration(text: &str) -> Option<Duration> {
  let text = text.trim();
  let (value, scale_ms) = if let Some(rest) = text.strip_suffix("ms") {
    (rest, 1)
  } else if let Some(rest) = text.strip_suffix('s') {
    (rest, 1_000)
  } else if let Some(rest) = text.strip_suffix('m') {
    (rest, 60_000)
  } else {
    (text, 1_000)
  };

  let value: u64 = value.trim().parse().ok()?;
  Some(Duration::from_millis(value.checked_mul(scale_ms)?))
}

/// Render a duration the way `parse_duration` would read it back.
fn format_duration(duration: Duration) -> String {
  let ms = duration.as_millis();
  if ms.is_multiple_of(1_000) {
    format!("{}s", ms / 1_000)
  } else {
    format!("{}ms", ms)
  }
}

/// One process definition borrowed from the text it was parsed out of.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessDef<'a> {
  pub name: &'a str,
  pub cmd: &'a str,
  pub flags: Flags,
}

impl ProcessDef<'_> {
  pub fn to_spec(&self) -> ProcessSpec {
    ProcessSpec {
      name: self.name.to_string(),
      cmd: self.cmd.to_string(),
      flags: self.flags,
    }
  }
}

/// An owned process definition, for handing to the manager.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessSpec {
  pub name: String,
  pub cmd: String,
  pub flags: Flags,
}

/// Split a name that may carry flags into `(name, flags)`.
///
/// `web` yields no flags; `web(once, optional)` yields both. Errors describe
/// what is wrong with the name alone, so callers can prefix them with a line
/// number or an argument name.
pub fn parse_name(head: &str) -> Result<(&str, Flags), String> {
  let head = head.trim();

  let Some(open) = head.find(FLAGS_OPEN) else {
    if head.contains(FLAGS_CLOSE) {
      return Err(format!("has an unmatched '{}' in its name", FLAGS_CLOSE));
    }
    return Ok((head, Flags::default()));
  };

  if !head.ends_with(FLAGS_CLOSE) {
    return Err(format!("has an unclosed '{}' in its flag list", FLAGS_OPEN));
  }

  let name = head[..open].trim();
  let body = head[open + FLAGS_OPEN.len_utf8()..head.len() - FLAGS_CLOSE.len_utf8()].trim();

  let mut flags = Flags::default();
  if body.is_empty() {
    return Ok((name, flags));
  }

  for token in body.split(',') {
    let token = token.trim();
    if token.is_empty() {
      return Err(format!(
        "has an empty flag (expected one of: {})",
        KNOWN_FLAGS
      ));
    }
    apply_flag(&mut flags, token)?;
  }

  Ok((name, flags))
}

/// Fold a single flag word into `flags`, rejecting unknown, repeated, and
/// mutually exclusive run modes.
///
/// Flag names are matched case-insensitively, and `_` reads the same as `-`
/// so both `allow-failure` and `allow_failure` work.
fn apply_flag(flags: &mut Flags, token: &str) -> Result<(), String> {
  let normalized = token.to_ascii_lowercase().replace('_', "-");
  let (key, value) = match normalized.split_once('=') {
    Some((key, value)) => (key.trim(), Some(value.trim())),
    None => (normalized.as_str(), None),
  };

  // Flags that carry a value are handled first, so `delay` without one is a
  // clear error rather than an unknown flag.
  match (key, value) {
    (DELAY, Some(value)) => {
      if flags.delay.is_some() {
        return Err(format!("repeats the flag {:?}", DELAY));
      }
      let Some(delay) = parse_duration(value) else {
        return Err(format!(
          "has an invalid {} {:?} (expected a duration like 500ms, 2s or 1m)",
          DELAY, value
        ));
      };
      flags.delay = Some(delay);
      return Ok(());
    }
    (RETRIES, Some(value)) => {
      if flags.retries.is_some() {
        return Err(format!("repeats the flag {:?}", RETRIES));
      }
      let Ok(retries) = value.parse::<u32>() else {
        return Err(format!(
          "has an invalid {} {:?} (expected a whole number)",
          RETRIES, value
        ));
      };
      flags.retries = Some(retries);
      return Ok(());
    }
    (DELAY | RETRIES, None) => {
      return Err(format!("needs a value for {:?}, as in {}=2", key, key));
    }
    (_, Some(_)) => {
      return Err(format!(
        "has an unknown flag {:?} (expected one of: {})",
        token, KNOWN_FLAGS
      ));
    }
    (_, None) => {}
  }

  let on_exit = match key {
    OPTIONAL => {
      if flags.optional {
        return Err(format!("repeats the flag {:?}", token));
      }
      flags.optional = true;
      return Ok(());
    }
    MUTED => {
      if flags.muted {
        return Err(format!("repeats the flag {:?}", token));
      }
      flags.muted = true;
      return Ok(());
    }
    ALLOW_FAILURE => {
      if flags.allow_failure {
        return Err(format!("repeats the flag {:?}", token));
      }
      flags.allow_failure = true;
      return Ok(());
    }
    ONCE => OnExit::Ignore,
    RESTART => OnExit::Restart,
    STOP => OnExit::Stop,
    _ => {
      return Err(format!(
        "has an unknown flag {:?} (expected one of: {})",
        token, KNOWN_FLAGS
      ));
    }
  };

  match flags.on_exit {
    Some(existing) if existing == on_exit => Err(format!("repeats the flag {:?}", token)),
    Some(_) => Err(format!(
      "has conflicting run modes (use only one of: {}, {}, {})",
      ONCE, RESTART, STOP
    )),
    None => {
      flags.on_exit = Some(on_exit);
      Ok(())
    }
  }
}

/// Parse a single `name: command` line (flags optional).
///
/// The error is a bare phrase like `"doesn't have a name"` so that both the
/// Procfile reader and `--run` can wrap it in their own context.
pub fn parse_line(line: &str) -> Result<ProcessDef<'_>, String> {
  let Some((head, cmd)) = line.split_once(NAME_CMD_SEPARATOR) else {
    return Err("should contain name and command".to_string());
  };

  let (name, flags) = parse_name(head)?;
  let cmd = cmd.trim();

  if name.is_empty() {
    return Err("doesn't have a name".to_string());
  }

  if cmd.is_empty() {
    return Err("doesn't have a command".to_string());
  }

  Ok(ProcessDef { name, cmd, flags })
}

/// Parse Procfile lines into process definitions.
/// If `names` is non-empty, only processes matching those names are returned.
/// Lines starting with `#` are comments.
pub fn parse<'a>(input: &'a str, names: &[String]) -> Result<Vec<ProcessDef<'a>>, String> {
  let mut result = Vec::new();
  let mut errors = Vec::new();

  for (n, line) in input.lines().enumerate() {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with(COMMENT_PREFIX) {
      continue;
    }

    let n = n + 1;

    match parse_line(line) {
      Ok(def) => {
        if !names.is_empty() && !names.iter().any(|f| f == def.name) {
          continue;
        }
        result.push(def);
      }
      Err(reason) => errors.push(format!("Line {} {}:\n> {}", n, reason, line)),
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

  fn names_and_cmds<'a>(defs: &[ProcessDef<'a>]) -> Vec<(&'a str, &'a str)> {
    defs.iter().map(|d| (d.name, d.cmd)).collect()
  }

  #[test]
  fn parse_accepts_indented_comments() {
    let input = "  # comment\nweb: echo hi\n";
    let parsed = parse(input, &[]).unwrap();
    assert_eq!(names_and_cmds(&parsed), vec![("web", "echo hi")]);
  }

  #[test]
  fn parse_filters_by_names() {
    let input = "web: echo web\napi: echo api\nworker: echo worker\n";
    let names = vec!["web".to_string(), "worker".to_string()];
    let parsed = parse(input, &names).unwrap();
    assert_eq!(
      names_and_cmds(&parsed),
      vec![("web", "echo web"), ("worker", "echo worker")]
    );
  }

  #[test]
  fn parse_reads_flags_after_the_name() {
    let input = "process(optional): echo 123\nanother(once, optional): echo 321\n";
    let parsed = parse(input, &[]).unwrap();

    assert_eq!(
      names_and_cmds(&parsed),
      vec![("process", "echo 123"), ("another", "echo 321")]
    );
    assert_eq!(
      parsed[0].flags,
      Flags {
        optional: true,
        ..Flags::default()
      }
    );
    assert_eq!(
      parsed[1].flags,
      Flags {
        optional: true,
        on_exit: Some(OnExit::Ignore),
        ..Flags::default()
      }
    );
  }

  #[test]
  fn parse_name_accepts_every_run_mode() {
    for (text, expected) in [
      ("web(once)", OnExit::Ignore),
      ("web(restart)", OnExit::Restart),
      ("web(stop)", OnExit::Stop),
    ] {
      let (name, flags) = parse_name(text).unwrap();
      assert_eq!(name, "web");
      assert_eq!(flags.on_exit, Some(expected));
      assert!(!flags.optional);
    }
  }

  #[test]
  fn parse_name_is_case_insensitive_and_tolerates_spacing() {
    let (name, flags) = parse_name("  web ( Optional ,  ONCE ) ").unwrap();
    assert_eq!(name, "web");
    assert_eq!(
      flags,
      Flags {
        optional: true,
        on_exit: Some(OnExit::Ignore),
        ..Flags::default()
      }
    );
  }

  #[test]
  fn parse_name_without_flags_is_unchanged() {
    let (name, flags) = parse_name("web").unwrap();
    assert_eq!(name, "web");
    assert_eq!(flags, Flags::default());

    // An empty flag list is simply no flags.
    let (name, flags) = parse_name("web()").unwrap();
    assert_eq!(name, "web");
    assert_eq!(flags, Flags::default());
  }

  #[test]
  fn parse_name_rejects_bad_flag_lists() {
    for text in [
      "web(nope)",
      "web(once, restart)",
      "web(once, once)",
      "web(optional, optional)",
      "web(optional,)",
      "web(optional",
      "web)optional(",
      "web(muted, muted)",
      "web(allow-failure, allow_failure)",
      "web(delay)",
      "web(retries)",
      "web(delay=soon)",
      "web(delay=2h)",
      "web(retries=many)",
      "web(retries=-1)",
      "web(delay=1s, delay=2s)",
      "web(retries=1, retries=2)",
      "web(nope=1)",
    ] {
      assert!(
        parse_name(text).is_err(),
        "expected {:?} to be rejected",
        text
      );
    }
  }

  #[test]
  fn parse_reports_the_offending_line_for_a_bad_flag() {
    let input = "web: echo hi\nworker(nope): echo bye\n";
    let err = parse(input, &[]).unwrap_err();
    assert!(err.contains("Line 2"), "{}", err);
    assert!(err.contains("nope"), "{}", err);
  }

  #[test]
  fn flags_round_trip_through_a_suffix() {
    assert_eq!(Flags::default().suffix(), "");

    for text in [
      "(optional, once)",
      "(restart)",
      "(restart, retries=5)",
      "(delay=2s)",
      "(delay=500ms, muted)",
      "(once, allow-failure)",
      "(optional, stop, delay=1s, retries=0, muted, allow-failure)",
    ] {
      let declaration = format!("web{}", text);
      let (name, flags) = parse_name(&declaration).unwrap();
      assert_eq!(name, "web");
      assert_eq!(flags.suffix(), text, "round trip of {}", text);
    }
  }

  #[test]
  fn parse_name_reads_the_value_carrying_flags() {
    let (_, flags) = parse_name("web(delay=1500ms, retries=3)").unwrap();
    assert_eq!(flags.delay, Some(Duration::from_millis(1500)));
    assert_eq!(flags.retries, Some(3));

    // A bare number is seconds, and minutes are accepted too.
    assert_eq!(parse_duration("2"), Some(Duration::from_secs(2)));
    assert_eq!(parse_duration("2s"), Some(Duration::from_secs(2)));
    assert_eq!(parse_duration("1m"), Some(Duration::from_secs(60)));
    assert_eq!(parse_duration("250ms"), Some(Duration::from_millis(250)));
  }

  #[test]
  fn parse_name_reads_the_standalone_flags() {
    let (_, flags) = parse_name("web(muted, allow-failure)").unwrap();
    assert!(flags.muted);
    assert!(flags.allow_failure);

    // An underscore reads the same as a hyphen.
    let (_, flags) = parse_name("web(allow_failure)").unwrap();
    assert!(flags.allow_failure);
  }

  #[test]
  fn retries_of_zero_is_a_real_value_not_an_absence() {
    let (_, flags) = parse_name("web(retries=0)").unwrap();
    assert_eq!(flags.retries, Some(0));
  }

  #[test]
  fn parse_line_still_requires_a_name_and_a_command() {
    assert!(parse_line("  : echo hi").is_err());
    assert!(parse_line("web:   ").is_err());
    assert!(parse_line("no separator here").is_err());
    assert!(parse_line("(once): echo hi").is_err());
  }
}
