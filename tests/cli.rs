use std::io::Write;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

fn run_with_fixture(path: &str) -> String {
    // Pin HOME to a directory without a .claude.json so the usage-cache
    // fallback stays inert unless a test opts in.
    run_with_fixture_and_home(path, "tests/fixtures")
}

fn run_with_fixture_and_home(path: &str, home: &str) -> String {
    run_with_fixture_home_and_time(path, home, None)
}

fn run_with_fixture_and_time(path: &str, time: &str) -> String {
    run_with_fixture_home_and_time(path, "tests/fixtures", Some(time))
}

fn run_with_fixture_home_and_time(path: &str, home: &str, time: Option<&str>) -> String {
    let input = fs::read_to_string(path).expect("fixture should be readable");
    let mut command = Command::new(env!("CARGO_BIN_EXE_claude-status-line"));
    command
        .env("HOME", Path::new(env!("CARGO_MANIFEST_DIR")).join(home))
        // Pin the clock so time-remaining labels render deterministically.
        // 1784592000 = 2026-07-21T00:00:00Z.
        .env("CLAUDE_STATUS_LINE_NOW", "1784592000")
        // Shield the tests from a value set in the invoking shell.
        .env_remove("CLAUDE_STATUS_LINE_TIME");
    if let Some(time) = time {
        command.env("CLAUDE_STATUS_LINE_TIME", time);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("binary should start");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(input.as_bytes())
        .expect("fixture should be written to stdin");

    let output = child.wait_with_output().expect("binary should finish");
    assert!(output.status.success());

    String::from_utf8(output.stdout).expect("stdout should be utf-8")
}

#[test]
fn prints_status_line_from_minimal_json() {
    let stdout = run_with_fixture("tests/fixtures/minimal-status.json");

    let top_line = "\x1b[38;5;214mfeature-test\x1b[0m";
    let bottom_line = [
        "\x1b[48;5;24m\x1b[38;5;15m Opus|high \x1b[0m",
        "\x1b[48;5;220m\x1b[38;5;0m ctx 51% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 5h 24% \x1b[0m",
        "\x1b[48;5;196m\x1b[38;5;0m 7d 81% \x1b[0m",
    ]
    .join(" ");

    assert_eq!(stdout, format!("{top_line}\n{bottom_line}\n"));
}

#[test]
fn prints_status_line_from_schema_example() {
    let stdout = run_with_fixture("tests/fixtures/schema.json");

    let top_line = [
        "\x1b[38;5;15m/current/working/directory\x1b[0m",
        "\x1b[38;5;63m|",
        "\x1b[38;5;214mworktree-my-feature\x1b[0m",
    ]
    .join(" ");
    let bottom_line = [
        "\x1b[48;5;24m\x1b[38;5;15m Opus|high \x1b[0m",
        "\x1b[48;5;196m\x1b[38;5;0m ctx 81% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 4h12m 24% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 6d3h 42% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 1d3h Fable 13% \x1b[0m",
    ]
    .join(" ");

    assert_eq!(stdout, format!("{top_line}\n{bottom_line}\n"));
}

#[test]
fn hides_time_remaining_when_time_display_is_none() {
    let stdout = run_with_fixture_and_time("tests/fixtures/schema.json", "none");

    let bottom_line = [
        "\x1b[48;5;24m\x1b[38;5;15m Opus|high \x1b[0m",
        "\x1b[48;5;196m\x1b[38;5;0m ctx 81% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 5h 24% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 7d 42% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 7d Fable 13% \x1b[0m",
    ]
    .join(" ");

    assert!(
        stdout.ends_with(&format!("{bottom_line}\n")),
        "unexpected output: {stdout}"
    );
}

#[test]
fn shows_single_unit_when_time_display_is_short() {
    let stdout = run_with_fixture_and_time("tests/fixtures/schema.json", "short");

    let bottom_line = [
        "\x1b[48;5;24m\x1b[38;5;15m Opus|high \x1b[0m",
        "\x1b[48;5;196m\x1b[38;5;0m ctx 81% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 4h 24% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 6d 42% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 1d Fable 13% \x1b[0m",
    ]
    .join(" ");

    assert!(
        stdout.ends_with(&format!("{bottom_line}\n")),
        "unexpected output: {stdout}"
    );
}

#[test]
fn adds_model_scoped_segment_from_usage_cache() {
    let stdout =
        run_with_fixture_and_home("tests/fixtures/minimal-status.json", "tests/fixtures/home");

    let top_line = "\x1b[38;5;214mfeature-test\x1b[0m";
    let bottom_line = [
        "\x1b[48;5;24m\x1b[38;5;15m Opus|high \x1b[0m",
        "\x1b[48;5;220m\x1b[38;5;0m ctx 51% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 5h 24% \x1b[0m",
        "\x1b[48;5;196m\x1b[38;5;0m 7d 81% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 1d3h Fable 4% \x1b[0m",
    ]
    .join(" ");

    assert_eq!(stdout, format!("{top_line}\n{bottom_line}\n"));
}

#[test]
fn prints_nothing_when_no_segments_can_be_built() {
    let stdout = run_with_fixture("tests/fixtures/empty-status.json");

    assert_eq!(stdout, "");
}
