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
    let input = fs::read_to_string(path).expect("fixture should be readable");
    let mut child = Command::new(env!("CARGO_BIN_EXE_claude-status-line"))
        .env("HOME", Path::new(env!("CARGO_MANIFEST_DIR")).join(home))
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
        "\x1b[48;5;34m\x1b[38;5;0m 5h 24% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 7d 42% \x1b[0m",
        "\x1b[48;5;34m\x1b[38;5;0m 7d Fable 13% \x1b[0m",
    ]
    .join(" ");

    assert_eq!(stdout, format!("{top_line}\n{bottom_line}\n"));
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
        "\x1b[48;5;34m\x1b[38;5;0m 7d Fable 4% \x1b[0m",
    ]
    .join(" ");

    assert_eq!(stdout, format!("{top_line}\n{bottom_line}\n"));
}

#[test]
fn prints_nothing_when_no_segments_can_be_built() {
    let stdout = run_with_fixture("tests/fixtures/empty-status.json");

    assert_eq!(stdout, "");
}
