//! End-to-end tests over the real binary.
//!
//! These drive `hew` as a process, so they cover the parts the unit tests
//! cannot: config discovery relative to the working directory, exit codes, the
//! stdout/stderr split, and behaviour when the reader closes early.

#![expect(
    clippy::unwrap_used,
    reason = "integration test: a panic IS the failure signal. tests/ compiles without cfg(test), so clippy.toml's allow-*-in-tests switches do not reach this file and the exception must be stated explicitly. Kept minimal on purpose — allow_attributes_without_reason plus unfulfilled_lint_expectations turn an over-broad suppression into a hard error."
)]

use std::{
    fs,
    io::{self, BufRead as _, BufReader, Write as _},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

/// A minimal config exercising both serde enum forms and affix nesting.
const CONFIG: &str = r#"
[[sections]]
name = "timestamp"
attribute = "timestamp"
affixes = []

[[sections]]
name = "level"
attribute = "level"

[[sections.affixes]]
condition = { Equals = "INFO" }
prefix = "<g>"
suffix = "</g>"

[[sections.affixes]]
condition = "Always"
prefix = " "
suffix = " "

[[sections]]
name = "message"
attribute = "message"
affixes = []
"#;

/// A private working directory holding a `config.toml`.
///
/// `CARGO_TARGET_TMPDIR` is provided by cargo for integration tests, which is
/// why this needs no `tempfile` dev-dependency — keeping the dependency graph at
/// the three crates the binary actually uses.
fn workspace(name: &str, config: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.toml"), config).unwrap();
    dir
}

fn run_in(dir: &Path, input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hew"))
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    // A child that rejects its config exits without ever reading stdin, which
    // closes this pipe. That is the behaviour under test, not a test failure —
    // and it is timing-dependent, since a small input can land in the pipe
    // buffer before the child gets round to exiting. Instrumented builds shift
    // that timing, so treating BrokenPipe as fatal here makes the test flaky.
    if let Err(err) = stdin.write_all(input.as_bytes()) {
        assert!(
            err.kind() == io::ErrorKind::BrokenPipe,
            "writing test input failed: {err}"
        );
    }
    // Explicit: the child needs EOF before wait_with_output can return.
    drop(stdin);

    child.wait_with_output().unwrap()
}

fn stdout_of(out: &Output) -> &str {
    std::str::from_utf8(&out.stdout).unwrap()
}

/// Writes `input` to a running hew and tries to read one line back **without
/// closing stdin**, returning `None` if nothing arrives within `timeout`.
///
/// This is the only way to observe flushing: `run_in` closes stdin, and the
/// final flush at end of stream would make a buffered child look identical to a
/// flushing one. The read happens on a worker thread behind a channel because a
/// buffered child never produces the line at all, and a blocking `read_line` on
/// this thread would hang the suite instead of failing it.
#[expect(
    clippy::unwrap_in_result,
    reason = "the Option is the observation under test — no output within the timeout — not an error channel; a failure to spawn is still a panic like everywhere else in this file"
)]
fn first_line_while_running(
    dir: &Path,
    args: &[&str],
    input: &str,
    timeout: Duration,
) -> Option<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hew"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut line = String::new();
        let outcome = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        // Send failures are expected: on the buffered path this line only shows
        // up after the timeout has passed and the receiver may be gone.
        let _sent = sender.send(outcome);
    });

    stdin.write_all(input.as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Ok(0) at EOF leaves the string empty, which is not a line.
    let line = receiver
        .recv_timeout(timeout)
        .ok()
        .and_then(Result::ok)
        .filter(|line| !line.is_empty());

    // EOF ends the child's loop, which ends the reader's blocking read.
    drop(stdin);
    let status = child.wait().unwrap();
    reader.join().unwrap();
    assert!(status.success(), "child exited with {status}");

    line
}

#[test]
fn flush_streams_each_line_into_a_pipe() {
    let dir = workspace("flush", CONFIG);
    let line = first_line_while_running(
        &dir,
        &["--flush"],
        "{\"timestamp\":\"T\",\"level\":\"INFO\",\"message\":\"live\"}\n",
        Duration::from_secs(10),
    );

    assert_eq!(
        line.as_deref(),
        Some("T <g>INFO</g> live\n"),
        "--flush must push each line into the pipe as it is written, formatted \
         exactly as it would be on a terminal"
    );
}

#[test]
fn without_flush_a_pipe_stays_buffered() {
    // Documents the default rather than guaranteeing it: this asserts an
    // absence, so the window is kept short. If it ever turns flaky under a
    // loaded CI machine, lengthening the timeout only makes it slower — delete
    // it instead, the positive test above is the one that carries the feature.
    let dir = workspace("no-flush", CONFIG);
    let line = first_line_while_running(
        &dir,
        &[],
        "{\"timestamp\":\"T\",\"level\":\"INFO\",\"message\":\"held\"}\n",
        Duration::from_secs(1),
    );

    assert_eq!(
        line, None,
        "one short line must sit in the 64 KiB buffer until --flush asks otherwise"
    );
}

#[test]
fn an_unknown_argument_fails_cleanly() {
    let dir = workspace("bad-arg", CONFIG);
    let mut child = Command::new(env!("CARGO_BIN_EXE_hew"))
        .current_dir(&dir)
        .arg("--flsh")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();

    assert!(!out.status.success(), "a typo must not be ignored");
    assert!(out.stdout.is_empty(), "nothing may reach stdout");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--flsh") && stderr.contains("--flush"),
        "must name the offending argument and show the usage: {stderr}"
    );
}

#[test]
fn formats_a_structured_stream() {
    let dir = workspace("formats", CONFIG);
    let out = run_in(
        &dir,
        "{\"timestamp\":\"T\",\"level\":\"INFO\",\"message\":\"hello\"}\n",
    );

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_of(&out), "T <g>INFO</g> hello\n");
    assert!(
        out.stderr.is_empty(),
        "a successful run must not write to stderr"
    );
}

#[test]
fn a_mixed_stream_passes_unparseable_lines_through() {
    let dir = workspace("mixed", CONFIG);
    let input = concat!(
        "{\"timestamp\":\"T\",\"level\":\"INFO\",\"message\":\"one\"}\n",
        "plain text, not JSON\n",
        "{\"level\":\"WARN\"}\n",
        "[1,2,3]\n",
        "{\"timestamp\":\"T\",\"level\":\"ERROR\",\"message\":\"done\"}\n",
    );
    let out = run_in(&dir, input);

    assert!(out.status.success());
    assert_eq!(
        stdout_of(&out),
        concat!(
            "T <g>INFO</g> one\n",
            "plain text, not JSON\n",
            // Only `level` is present, so only `level` renders — with the spaces
            // from its Always affix, which is the whole separator budget of this
            // config.
            " WARN \n",
            "[1,2,3]\n",
            "T ERROR done\n",
        ),
        "every line must appear exactly once: formatted from whatever attributes \
         it carries, or verbatim when it carries none"
    );
}

#[test]
fn empty_input_succeeds_with_empty_output() {
    let dir = workspace("empty", CONFIG);
    let out = run_in(&dir, "");

    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn a_missing_config_fails_cleanly() {
    // A directory with no config.toml at all.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config");
    fs::create_dir_all(&dir).unwrap();
    // May legitimately not exist; a named binding rather than `let _`, which
    // let_underscore_drop rejects.
    let _removed = fs::remove_file(dir.join("config.toml"));

    let out = run_in(&dir, "{\"a\":1}\n");

    assert!(!out.status.success(), "a missing config must be fatal");
    assert!(out.stdout.is_empty(), "nothing may reach stdout");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("config.toml"),
        "must name the file: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "must be a diagnostic, not a panic backtrace: {stderr}"
    );
}

#[test]
fn a_malformed_config_fails_cleanly() {
    let dir = workspace("bad-config", "this is not toml [[[\n");
    let out = run_in(&dir, "{\"a\":1}\n");

    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("parse"),
        "must say it could not parse: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "must not be a panic: {stderr}"
    );
}

#[test]
fn the_shipped_config_parses() {
    // Guards against the repo's own config.toml drifting away from the schema.
    let repo_config = Path::new(env!("CARGO_MANIFEST_DIR")).join("config.toml");
    let source = fs::read_to_string(&repo_config).unwrap();
    let dir = workspace("shipped", &source);

    let out = run_in(
        &dir,
        "{\"timestamp\":\"T\",\"level\":\"INFO\",\"message\":\"m\"}\n",
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout_of(&out).contains("INFO"));
}

#[cfg(unix)]
#[test]
fn a_reader_that_closes_early_is_not_an_error() {
    // `hew | head -n1` must exit 0. Rust ignores SIGPIPE, so without explicit
    // BrokenPipe handling this would exit nonzero and print a spurious error.
    let dir = workspace("broken-pipe", CONFIG);
    let binary = env!("CARGO_BIN_EXE_hew");

    let script = format!(
        "for i in $(seq 1 20000); do \
           printf '{{\"timestamp\":\"T\",\"level\":\"INFO\",\"message\":\"%s\"}}\\n' \"$i\"; \
         done | '{binary}' 2>&1 | head -n 1"
    );

    let out = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .current_dir(&dir)
        .output()
        .unwrap();

    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("<g>INFO</g>"),
        "expected a formatted line, got: {text:?}"
    );
    assert!(
        !text.contains("Broken pipe") && !text.contains("panicked"),
        "no error may be reported when the reader goes away: {text:?}"
    );
}
