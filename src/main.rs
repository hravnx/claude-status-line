use std::{
    io::{self, Read},
    process::Command,
};

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_FG_BLACK: &str = "\x1b[38;5;0m";
const ANSI_FG_WHITE: &str = "\x1b[38;5;15m";
const ANSI_BG_BLUE: &str = "\x1b[48;5;24m";
const ANSI_BG_PURPLE: &str = "\x1b[48;5;60m";
const ANSI_BG_GREEN: &str = "\x1b[48;5;34m";
const ANSI_BG_YELLOW: &str = "\x1b[48;5;220m";
const ANSI_BG_RED: &str = "\x1b[48;5;196m";

fn format_percentage_segment(label: &str, value: f64) -> String {
    let bg_color = if value <= 50.0 {
        ANSI_BG_GREEN
    } else if value <= 80.0 {
        ANSI_BG_YELLOW
    } else {
        ANSI_BG_RED
    };

    format!(
        "{bg_color}{ANSI_FG_BLACK} {label} {}% {ANSI_RESET}",
        value.ceil()
    )
}

fn format_model_segment(model_name: &str, effort_level: &str) -> String {
    format!("{ANSI_BG_BLUE}{ANSI_FG_WHITE} {model_name}|{effort_level} {ANSI_RESET}")
}

fn format_branch_segment(branch: &str) -> String {
    format!("{ANSI_BG_PURPLE}{ANSI_FG_WHITE} {branch} {ANSI_RESET}")
}

fn percentage_at(json: &serde_json::Value, path: &[&str]) -> Option<f64> {
    path.iter()
        .try_fold(json, |value, key| value.get(*key))
        .and_then(|value| value.as_f64())
}

fn string_at<'a>(json: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(json, |value, key| value.get(*key))
        .and_then(|value| value.as_str())
}

fn git_branch(cwd: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", cwd, "branch", "--show-current"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

fn active_branch(json: &serde_json::Value) -> Option<String> {
    string_at(json, &["worktree", "branch"])
        .filter(|branch| !branch.is_empty())
        .map(str::to_string)
        .or_else(|| string_at(json, &["cwd"]).and_then(git_branch))
        .or_else(|| string_at(json, &["workspace", "current_dir"]).and_then(git_branch))
}

fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return;
    }

    let Ok(json) = serde_json::from_str::<serde_json::Value>(&input) else {
        return;
    };

    let mut segments = Vec::new();

    if let Some(branch) = active_branch(&json) {
        segments.push(format_branch_segment(&branch));
    }

    if let Some(value) = percentage_at(&json, &["context_window", "used_percentage"]) {
        segments.push(format_percentage_segment("ctx", value));
    }

    if let Some(value) = percentage_at(&json, &["rate_limits", "five_hour", "used_percentage"]) {
        segments.push(format_percentage_segment("5h", value));
    }

    if let Some(value) = percentage_at(&json, &["rate_limits", "seven_day", "used_percentage"]) {
        segments.push(format_percentage_segment("7d", value));
    }

    if let (Some(model_name), Some(effort_level)) = (
        string_at(&json, &["model", "display_name"]),
        string_at(&json, &["effort", "level"]),
    ) {
        segments.push(format_model_segment(model_name, effort_level));
    }

    if !segments.is_empty() {
        println!("{}", segments.join(" "));
    }
}
