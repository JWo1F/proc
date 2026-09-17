//! End-to-end tests that drive the real `proc` binary.
//!
//! Each test gets a throwaway directory holding its own Procfile, so the
//! default config path resolves without any flags. Every process used here
//! exits on its own within milliseconds — nothing sleeps or restarts, so the
//! suite stays deterministic.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_proc");

/// A directory with a Procfile in it, removed when the test ends.
struct Project {
  path: PathBuf,
}

impl Project {
  /// An empty directory — no Procfile at all.
  fn empty() -> Self {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("proc-cli-{}-{}", std::process::id(), unique));
    fs::create_dir_all(&path).expect("create temp project");
    Self { path }
  }

  fn with_procfile(contents: &str) -> Self {
    let project = Self::empty();
    fs::write(project.path.join("Procfile"), contents).expect("write Procfile");
    project
  }

  fn path(&self) -> &Path {
    &self.path
  }

  fn run(&self, args: &[&str]) -> Run {
    let output = Command::new(BIN)
      .args(args)
      .current_dir(&self.path)
      .stdin(Stdio::null())
      .output()
      .expect("run proc");
    Run::from(output)
  }
}

impl Drop for Project {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.path);
  }
}

/// One finished run, with its streams decoded.
struct Run {
  code: Option<i32>,
  stdout: String,
  stderr: String,
}

impl From<Output> for Run {
  fn from(output: Output) -> Self {
    Self {
      code: output.status.code(),
      stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
      stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
  }
}

impl Run {
  /// Both streams together, for assertions that do not care which one a
  /// message landed on.
  fn output(&self) -> String {
    format!("{}{}", self.stdout, self.stderr)
  }

  #[track_caller]
  fn assert_code(&self, expected: i32) -> &Self {
    assert_eq!(
      self.code,
      Some(expected),
      "expected exit {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
      expected,
      self.stdout,
      self.stderr
    );
    self
  }

  #[track_caller]
  fn assert_contains(&self, needle: &str) -> &Self {
    assert!(
      self.output().contains(needle),
      "expected {:?} in output\n--- stdout ---\n{}\n--- stderr ---\n{}",
      needle,
      self.stdout,
      self.stderr
    );
    self
  }

  #[track_caller]
  fn assert_lacks(&self, needle: &str) -> &Self {
    assert!(
      !self.output().contains(needle),
      "did not expect {:?} in output\n--- stdout ---\n{}\n--- stderr ---\n{}",
      needle,
      self.stdout,
      self.stderr
    );
    self
  }
}

// ---------------------------------------------------------------- check

#[test]
fn check_accepts_a_valid_procfile() {
  let project = Project::with_procfile("web: echo hi\nworker: echo bye\n");
  project
    .run(&["check"])
    .assert_code(0)
    .assert_contains("2 processes OK")
    .assert_contains("web: echo hi");
}

#[test]
fn check_echoes_flags_back() {
  let project =
    Project::with_procfile("web(delay:2s, retries:3): echo hi\nseed(optional, once): echo go\n");
  project
    .run(&["check"])
    .assert_code(0)
    .assert_contains("web(delay:2s, retries:3): echo hi")
    .assert_contains("seed(optional, once): echo go");
}

#[test]
fn check_skips_comments_and_blank_lines() {
  let project = Project::with_procfile("# a comment\n\nweb: echo hi\n\n  # indented comment\n");
  project
    .run(&["check"])
    .assert_code(0)
    .assert_contains("1 process");
}

#[test]
fn check_rejects_an_unknown_flag() {
  let project = Project::with_procfile("web(nope): echo hi\n");
  project
    .run(&["check"])
    .assert_code(2)
    .assert_contains("unknown flag")
    .assert_contains("nope");
}

#[test]
fn check_rejects_the_old_equals_separator() {
  let project = Project::with_procfile("web(delay=2s): echo hi\n");
  project
    .run(&["check"])
    .assert_code(2)
    .assert_contains("delay:<duration>");
}

#[test]
fn check_rejects_conflicting_run_modes() {
  let project = Project::with_procfile("web(once, restart): echo hi\n");
  project
    .run(&["check"])
    .assert_code(2)
    .assert_contains("conflicting run modes");
}

#[test]
fn check_rejects_a_line_without_a_command() {
  let project = Project::with_procfile("web:\n");
  project.run(&["check"]).assert_code(2);
}

#[test]
fn check_reports_a_missing_procfile() {
  let project = Project::empty();
  let run = project.run(&["check"]);
  run.assert_code(2).assert_contains("Procfile");
  assert!(
    !run.output().contains("os error"),
    "should not leak the raw OS error: {}",
    run.output()
  );
}

#[test]
fn check_reads_the_path_given_by_config() {
  let project = Project::with_procfile("web: echo from-default\n");
  fs::write(
    project.path().join("other.Procfile"),
    "api: echo from-other\n",
  )
  .unwrap();
  project
    .run(&["check", "-c", "other.Procfile"])
    .assert_code(0)
    .assert_contains("api: echo from-other")
    .assert_lacks("from-default");
}

// ----------------------------------------------------------------- list

#[test]
fn list_prints_names_and_commands() {
  let project = Project::with_procfile("web: echo hi\nworker: echo bye\n");
  project
    .run(&["list"])
    .assert_code(0)
    .assert_contains("web")
    .assert_contains("echo hi")
    .assert_contains("worker")
    .assert_contains("echo bye");
}

#[test]
fn list_shows_the_flags_a_process_carries() {
  let project = Project::with_procfile("seed(optional, allow-failure): echo go\n");
  project
    .run(&["list"])
    .assert_code(0)
    .assert_contains("seed(optional, allow-failure)");
}

// ------------------------------------------------------------- running

#[test]
fn a_process_runs_and_its_output_is_prefixed() {
  let project = Project::with_procfile("greeter: echo marker-one\n");
  project
    .run(&["-c", "Procfile"])
    .assert_code(0)
    .assert_contains("greeter")
    .assert_contains("marker-one");
}

#[test]
fn every_process_runs_when_exits_are_ignored() {
  let project = Project::with_procfile("one: echo marker-one\ntwo: echo marker-two\n");
  project
    .run(&["-c", "Procfile", "--on-exit", "ignore"])
    .assert_code(0)
    .assert_contains("marker-one")
    .assert_contains("marker-two");
}

#[test]
fn naming_processes_narrows_the_run() {
  let project = Project::with_procfile("one: echo marker-one\ntwo: echo marker-two\n");
  project
    .run(&["one"])
    .assert_code(0)
    .assert_contains("marker-one")
    .assert_lacks("marker-two");
}

#[test]
fn a_failing_process_fails_the_run() {
  let project = Project::with_procfile("boom: exit 3\n");
  let run = project.run(&["-c", "Procfile"]);
  assert_ne!(
    run.code,
    Some(0),
    "expected a non-zero exit: {}",
    run.output()
  );
}

#[test]
fn allow_failure_keeps_the_run_green() {
  let project = Project::with_procfile("boom(allow-failure): exit 3\n");
  project.run(&["-c", "Procfile"]).assert_code(0);
}

#[test]
fn an_optional_process_stays_out_until_it_is_asked_for() {
  let project = Project::with_procfile("one: echo marker-one\nseed(optional): echo marker-seed\n");

  project
    .run(&["-c", "Procfile", "--on-exit", "ignore"])
    .assert_code(0)
    .assert_contains("marker-one")
    .assert_lacks("marker-seed");

  project
    .run(&["-c", "Procfile", "--on-exit", "ignore", "-e", "seed"])
    .assert_code(0)
    .assert_contains("marker-one")
    .assert_contains("marker-seed");

  project
    .run(&["-c", "Procfile", "--on-exit", "ignore", "-E"])
    .assert_code(0)
    .assert_contains("marker-seed");
}

#[test]
fn muted_output_stays_off_the_terminal() {
  let project = Project::with_procfile("loud: echo marker-loud\nquiet(muted): echo marker-quiet\n");
  project
    .run(&["-c", "Procfile", "--on-exit", "ignore"])
    .assert_code(0)
    .assert_contains("marker-loud")
    .assert_lacks("marker-quiet");
}

#[test]
fn silent_suppresses_every_line() {
  let project = Project::with_procfile("loud: echo marker-loud\n");
  project
    .run(&["-c", "Procfile", "-q"])
    .assert_code(0)
    .assert_lacks("marker-loud");
}

#[test]
fn no_system_hides_the_lifecycle_messages() {
  let project = Project::with_procfile("one: echo marker-one\n");
  let noisy = project.run(&["-c", "Procfile"]);
  noisy.assert_code(0).assert_contains("marker-one");

  project
    .run(&["-c", "Procfile", "--no-system"])
    .assert_code(0)
    .assert_contains("marker-one")
    .assert_lacks("Spawned");
}

#[test]
fn timestamps_prefix_each_line() {
  let project = Project::with_procfile("one: echo marker-one\n");
  let run = project.run(&["-c", "Procfile", "-T"]);
  run.assert_code(0).assert_contains("marker-one");
  let stamped = run
    .output()
    .lines()
    .any(|line| line.contains("marker-one") && line.contains(':'));
  assert!(stamped, "expected a timestamped line:\n{}", run.output());
}

// ------------------------------------------------------- inline entries

#[test]
fn inline_entries_run_without_a_procfile() {
  let project = Project::empty();
  project
    .run(&["-r", "inline: echo marker-inline"])
    .assert_code(0)
    .assert_contains("marker-inline");
}

#[test]
fn inline_entries_take_the_same_flags_as_a_file_line() {
  let project = Project::empty();
  project
    .run(&["-r", "inline(allow-failure): exit 3"])
    .assert_code(0);
}

#[test]
fn a_malformed_inline_entry_is_rejected() {
  let project = Project::empty();
  project
    .run(&["-r", "no separator here"])
    .assert_code(2)
    .assert_contains("Invalid --run entry");
}

#[test]
fn inline_entries_join_the_procfile_when_a_config_is_given() {
  let project = Project::with_procfile("one: echo marker-one\n");
  project
    .run(&[
      "--on-exit",
      "ignore",
      "-c",
      "Procfile",
      "-r",
      "two: echo marker-two",
    ])
    .assert_code(0)
    .assert_contains("marker-one")
    .assert_contains("marker-two");
}

// --------------------------------------------- pipe mode vs. explicit intent

#[test]
fn naming_a_process_beats_redirected_stdin() {
  // stdin is /dev/null here, exactly as it is under CI or a Makefile.
  let project = Project::with_procfile("one: echo marker-one\n");
  project
    .run(&["one"])
    .assert_code(0)
    .assert_contains("marker-one")
    .assert_lacks("Reading from pipe");
}

#[test]
fn an_inline_entry_beats_redirected_stdin() {
  let project = Project::empty();
  project
    .run(&["-r", "inline: echo marker-inline"])
    .assert_code(0)
    .assert_contains("marker-inline")
    .assert_lacks("Reading from pipe");
}

// ----------------------------------------------------------- pipe mode

#[test]
fn pipe_mode_renders_stdin() {
  let project = Project::empty();
  let mut child = Command::new(BIN)
    .current_dir(project.path())
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("spawn proc");

  child
    .stdin
    .take()
    .expect("stdin")
    .write_all(b"marker-piped\nsecond-line\n")
    .expect("write stdin");

  let run = Run::from(child.wait_with_output().expect("wait for proc"));
  run
    .assert_code(0)
    .assert_contains("stdin")
    .assert_contains("marker-piped")
    .assert_contains("second-line");
}

// -------------------------------------------------------------- basics

#[test]
fn version_matches_the_crate() {
  let project = Project::empty();
  project
    .run(&["--version"])
    .assert_code(0)
    .assert_contains(env!("CARGO_PKG_VERSION"));
}

#[test]
fn help_documents_the_flag_syntax() {
  let project = Project::empty();
  project
    .run(&["--help"])
    .assert_code(0)
    .assert_contains("delay:<dur>")
    .assert_contains("retries:<n>")
    .assert_contains("allow-failure");
}

#[test]
fn the_web_flag_is_gone() {
  let project = Project::with_procfile("one: echo hi\n");
  let run = project.run(&["-w"]);
  assert_ne!(run.code, Some(0), "-w should no longer be accepted");
}
