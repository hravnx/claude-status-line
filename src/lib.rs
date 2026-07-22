use std::path::PathBuf;
use std::process::Command;

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_FG_BLACK: &str = "\x1b[38;5;0m";
const ANSI_FG_WHITE: &str = "\x1b[38;5;15m";
const ANSI_FG_ORANGE: &str = "\x1b[38;5;214m";
const ANSI_FG_PURPLE: &str = "\x1b[38;5;63m";
const ANSI_FG_DARK_RED: &str = "\x1b[38;5;124m";
const ANSI_BG_BLUE: &str = "\x1b[48;5;24m";
const ANSI_BG_GREEN: &str = "\x1b[48;5;34m";
const ANSI_BG_YELLOW: &str = "\x1b[48;5;220m";
const ANSI_BG_RED: &str = "\x1b[48;5;196m";

pub fn format_status_line(json: &serde_json::Value) -> Option<String> {
    format_status_line_with(json, now(), time_display())
}

fn format_status_line_with(json: &serde_json::Value, now: i64, time: TimeDisplay) -> Option<String> {
    let lines: Vec<String> = status_lines(json, now, time)
        .into_iter()
        .filter(|segments| !segments.is_empty())
        .map(|segments| segments.join(" "))
        .collect();

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Current time as Unix epoch seconds. Honors `CLAUDE_STATUS_LINE_NOW` (epoch
/// seconds) when set, which lets the end-to-end tests pin a deterministic clock
/// across the process boundary; production leaves it unset and reads the system
/// clock.
fn now() -> i64 {
    if let Some(raw) = std::env::var_os("CLAUDE_STATUS_LINE_NOW")
        && let Some(secs) = raw.to_str().and_then(|s| s.trim().parse::<i64>().ok())
    {
        return secs;
    }

    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// How rate-limit window labels render the time remaining until the window
/// resets, controlled by `CLAUDE_STATUS_LINE_TIME`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TimeDisplay {
    /// Two largest non-zero units (`4h12m`). Default.
    Normal,
    /// Static labels only (`5h`, `7d`), no countdown.
    None,
    /// Single largest unit (`4h`).
    Short,
}

fn time_display() -> TimeDisplay {
    time_display_from(std::env::var("CLAUDE_STATUS_LINE_TIME").ok().as_deref())
}

/// Unset, `normal`, or anything unrecognized means `Normal`, so a typo
/// degrades to the default rather than hiding information.
fn time_display_from(value: Option<&str>) -> TimeDisplay {
    match value.map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("none") => TimeDisplay::None,
        Some(v) if v.eq_ignore_ascii_case("short") => TimeDisplay::Short,
        _ => TimeDisplay::Normal,
    }
}

/// Two rows: workspace dir and branch on top, usage and model below.
/// Claude Code renders each line of output as its own status row.
fn status_lines(json: &serde_json::Value, now: i64, time: TimeDisplay) -> [Vec<String>; 2] {
    let mut top = Vec::new();

    if let Some(dir) = workspace_dir(json) {
        top.push(format_dir_segment(&dir));
        top.push(format!("{ANSI_FG_PURPLE}|"));
        top.push(match active_branch(json) {
            Some(branch) => format_branch_segment(&branch),
            None => format!("{ANSI_FG_DARK_RED}<no git branch>{ANSI_RESET}"),
        });
    } else if let Some(branch) = active_branch(json) {
        top.push(format_branch_segment(&branch));
    }

    let mut segments = Vec::new();

    if let (Some(model_name), Some(effort_level)) = (
        string_at(json, &["model", "display_name"]),
        string_at(json, &["effort", "level"]),
    ) {
        segments.push(format_model_segment(model_name, effort_level));
    }

    add_percentage_segment(
        &mut segments,
        json,
        "ctx",
        &["context_window", "used_percentage"],
    );
    add_window_segment(
        &mut segments,
        json,
        "5h",
        &["rate_limits", "five_hour", "used_percentage"],
        &["rate_limits", "five_hour", "resets_at"],
        now,
        time,
    );
    add_window_segment(
        &mut segments,
        json,
        "7d",
        &["rate_limits", "seven_day", "used_percentage"],
        &["rate_limits", "seven_day", "resets_at"],
        now,
        time,
    );

    for (model_name, value, resets_at) in model_scoped_limits(json) {
        let resets_at = resets_at.as_deref().and_then(parse_iso8601);
        let label = match resets_at.and_then(|resets_at| remaining_label(time, resets_at, now)) {
            Some(remaining) => format!("{remaining} {model_name}"),
            None => format!("7d {model_name}"),
        };
        segments.push(format_percentage_segment(&label, value));
    }

    [top, segments]
}

fn add_percentage_segment(
    segments: &mut Vec<String>,
    json: &serde_json::Value,
    label: &str,
    path: &[&str],
) {
    if let Some(value) = percentage_at(json, path) {
        segments.push(format_percentage_segment(label, value));
    }
}

/// Like `add_percentage_segment`, but for rate-limit windows: the label shows
/// the time remaining until the window resets (from `resets_path`) instead of a
/// fixed name. Falls back to `fallback_label` when no reset time is available.
fn add_window_segment(
    segments: &mut Vec<String>,
    json: &serde_json::Value,
    fallback_label: &str,
    percentage_path: &[&str],
    resets_path: &[&str],
    now: i64,
    time: TimeDisplay,
) {
    let Some(value) = percentage_at(json, percentage_path) else {
        return;
    };

    let label = epoch_at(json, resets_path)
        .and_then(|resets_at| remaining_label(time, resets_at, now))
        .unwrap_or_else(|| fallback_label.to_string());
    segments.push(format_percentage_segment(&label, value));
}

/// Countdown text for a window resetting at `resets_at`, per the display mode.
/// `None` means the static fallback label should be used instead.
fn remaining_label(time: TimeDisplay, resets_at: i64, now: i64) -> Option<String> {
    match time {
        TimeDisplay::None => None,
        TimeDisplay::Normal => Some(format_remaining(resets_at - now)),
        TimeDisplay::Short => Some(format_remaining_short(resets_at - now)),
    }
}

/// Compact remaining time using the two largest non-zero units (e.g. `4h12m`,
/// `6d3h`, `47m`). A zero secondary unit is dropped (`6d`, `1h`). Anything at or
/// below one minute — including already-expired windows — renders `<1m`.
fn format_remaining(secs: i64) -> String {
    if secs <= 60 {
        return "<1m".to_string();
    }

    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;

    if days > 0 {
        if hours > 0 {
            format!("{days}d{hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        if minutes > 0 {
            format!("{hours}h{minutes}m")
        } else {
            format!("{hours}h")
        }
    } else {
        format!("{minutes}m")
    }
}

/// Like `format_remaining`, but only the single largest non-zero unit
/// (e.g. `4h`, `6d`, `47m`).
fn format_remaining_short(secs: i64) -> String {
    if secs <= 60 {
        return "<1m".to_string();
    }

    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;

    if days > 0 {
        format!("{days}d")
    } else if hours > 0 {
        format!("{hours}h")
    } else {
        format!("{minutes}m")
    }
}

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
    format!("{ANSI_FG_ORANGE}{branch}{ANSI_RESET}")
}

fn format_dir_segment(dir: &str) -> String {
    format!("{ANSI_FG_WHITE}{dir}{ANSI_RESET}")
}

fn workspace_dir(json: &serde_json::Value) -> Option<String> {
    workspace_dir_with(json, home_dir())
}

fn workspace_dir_with(json: &serde_json::Value, home: Option<PathBuf>) -> Option<String> {
    let dir = string_at(json, &["workspace", "current_dir"])
        .or_else(|| string_at(json, &["cwd"]))
        .filter(|dir| !dir.is_empty())?;

    Some(tildify(dir, home.as_deref().and_then(|home| home.to_str())))
}

fn home_dir() -> Option<PathBuf> {
    // HOME on unix; USERPROFILE on Windows.
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn tildify(path: &str, home: Option<&str>) -> String {
    let Some(home) = home.map(|home| home.trim_end_matches(['/', '\\'])) else {
        return path.to_string();
    };

    if home.is_empty() {
        return path.to_string();
    }

    match path.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with(['/', '\\']) => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// Per-model weekly rate limits (e.g. Fable), reported separately from the
/// all-models `seven_day` bucket.
///
/// Preferred source is `rate_limits.model_scoped` in the status JSON. Claude
/// Code does not emit that field yet, so until it does, fall back to the
/// usage snapshot it caches in `~/.claude.json`.
fn model_scoped_limits(json: &serde_json::Value) -> Vec<(String, f64, Option<String>)> {
    model_scoped_limits_with(json, load_usage_cache)
}

fn model_scoped_limits_with<F>(
    json: &serde_json::Value,
    load_usage_cache: F,
) -> Vec<(String, f64, Option<String>)>
where
    F: Fn() -> Option<serde_json::Value>,
{
    if let Some(limits) = payload_model_scoped(json) {
        return limits;
    }

    // Only consult the cache for sessions that report rate limits at all;
    // API-key and third-party provider sessions have no plan limits.
    if json.get("rate_limits").is_none() {
        return Vec::new();
    }

    load_usage_cache()
        .map(|cache| cached_model_scoped(&cache))
        .unwrap_or_default()
}

fn payload_model_scoped(json: &serde_json::Value) -> Option<Vec<(String, f64, Option<String>)>> {
    let entries = json.get("rate_limits")?.get("model_scoped")?.as_array()?;

    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let name = string_at(entry, &["display_name"])?;
                let value = percentage_at(entry, &["utilization"])?;
                let resets_at = string_at(entry, &["resets_at"]).map(str::to_string);
                Some((name.to_string(), value, resets_at))
            })
            .collect(),
    )
}

fn cached_model_scoped(cache: &serde_json::Value) -> Vec<(String, f64, Option<String>)> {
    let limits = cache
        .get("cachedUsageUtilization")
        .and_then(|value| value.get("utilization"))
        .and_then(|value| value.get("limits"))
        .and_then(|value| value.as_array());

    limits
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            if string_at(entry, &["kind"])? != "weekly_scoped" {
                return None;
            }
            let name = string_at(entry, &["scope", "model", "display_name"])?;
            let value = percentage_at(entry, &["percent"])?;
            let resets_at = string_at(entry, &["resets_at"]).map(str::to_string);
            Some((name.to_string(), value, resets_at))
        })
        .collect()
}

fn load_usage_cache() -> Option<serde_json::Value> {
    let path = PathBuf::from(std::env::var_os("HOME")?).join(".claude.json");
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
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

/// Reads an integer Unix-epoch value at `path`. `five_hour`/`seven_day`
/// `resets_at` are reported as integer seconds; `null`, missing, or non-integer
/// values yield `None`.
fn epoch_at(json: &serde_json::Value, path: &[&str]) -> Option<i64> {
    path.iter()
        .try_fold(json, |value, key| value.get(*key))
        .and_then(|value| value.as_i64())
}

/// Parses the RFC-3339 subset Claude Code emits for `model_scoped` reset times
/// (`YYYY-MM-DDTHH:MM:SS[.fraction][Z|±HH:MM|±HHMM]`) into Unix epoch seconds.
/// Fractional seconds are ignored. Any malformed input yields `None`, which
/// callers treat as "no reset time" and fall back to the static label.
fn parse_iso8601(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();

    let year: i64 = s.get(0..4)?.parse().ok()?;
    if bytes.get(4)? != &b'-' {
        return None;
    }
    let month: i64 = s.get(5..7)?.parse().ok()?;
    if bytes.get(7)? != &b'-' {
        return None;
    }
    let day: i64 = s.get(8..10)?.parse().ok()?;
    match bytes.get(10)? {
        b'T' | b't' | b' ' => {}
        _ => return None,
    }
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    if bytes.get(13)? != &b':' {
        return None;
    }
    let minute: i64 = s.get(14..16)?.parse().ok()?;
    if bytes.get(16)? != &b':' {
        return None;
    }
    let second: i64 = s.get(17..19)?.parse().ok()?;

    // Skip an optional ".fraction", then interpret the timezone token.
    let rest = s.get(19..)?;
    let tz = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset_secs = parse_tz_offset(tz)?;

    let days = days_from_civil(year, month, day)?;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second - offset_secs)
}

/// `Z`/`z` -> 0; `+HH:MM`/`-HH:MM`/`+HHMM` -> seconds east of UTC.
fn parse_tz_offset(tz: &str) -> Option<i64> {
    match tz.as_bytes().first()? {
        b'Z' | b'z' => Some(0),
        sign @ (b'+' | b'-') => {
            let hours: i64 = tz.get(1..3)?.parse().ok()?;
            let minute_start = if tz.as_bytes().get(3) == Some(&b':') {
                4
            } else {
                3
            };
            let minutes: i64 = tz.get(minute_start..minute_start + 2)?.parse().ok()?;
            let magnitude = hours * 3_600 + minutes * 60;
            Some(if *sign == b'-' { -magnitude } else { magnitude })
        }
        _ => None,
    }
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01 in the proleptic
/// Gregorian calendar. Returns `None` for out-of-range months or days.
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let year_of_era = y - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
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

fn active_branch_with<F>(json: &serde_json::Value, git_branch: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    string_at(json, &["worktree", "branch"])
        .filter(|branch| !branch.is_empty())
        .map(str::to_string)
        .or_else(|| string_at(json, &["cwd"]).and_then(&git_branch))
        .or_else(|| string_at(json, &["workspace", "current_dir"]).and_then(git_branch))
}

fn active_branch(json: &serde_json::Value) -> Option<String> {
    active_branch_with(json, git_branch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_percentage_segments_with_threshold_colors() {
        assert_eq!(
            format_percentage_segment("ctx", 50.0),
            "\x1b[48;5;34m\x1b[38;5;0m ctx 50% \x1b[0m"
        );
        assert_eq!(
            format_percentage_segment("ctx", 50.1),
            "\x1b[48;5;220m\x1b[38;5;0m ctx 51% \x1b[0m"
        );
        assert_eq!(
            format_percentage_segment("ctx", 80.0),
            "\x1b[48;5;220m\x1b[38;5;0m ctx 80% \x1b[0m"
        );
        assert_eq!(
            format_percentage_segment("ctx", 80.1),
            "\x1b[48;5;196m\x1b[38;5;0m ctx 81% \x1b[0m"
        );
    }

    #[test]
    fn formats_model_and_branch_segments() {
        assert_eq!(
            format_model_segment("Opus", "high"),
            "\x1b[48;5;24m\x1b[38;5;15m Opus|high \x1b[0m"
        );
        assert_eq!(format_branch_segment("main"), "\x1b[38;5;214mmain\x1b[0m");
        assert_eq!(
            format_dir_segment("~/dev/project"),
            "\x1b[38;5;15m~/dev/project\x1b[0m"
        );
    }

    #[test]
    fn reads_nested_percentages_and_strings() {
        let status = json!({
            "context_window": { "used_percentage": 42.5 },
            "model": { "display_name": "Sonnet" }
        });

        assert_eq!(
            percentage_at(&status, &["context_window", "used_percentage"]),
            Some(42.5)
        );
        assert_eq!(
            string_at(&status, &["model", "display_name"]),
            Some("Sonnet")
        );
        assert_eq!(percentage_at(&status, &["missing"]), None);
        assert_eq!(
            string_at(&status, &["context_window", "used_percentage"]),
            None
        );
    }

    #[test]
    fn builds_status_line_from_json_fields() {
        let status = json!({
            "workspace": { "current_dir": "/srv/example/project" },
            "worktree": { "branch": "feature" },
            "context_window": { "used_percentage": 50.1 },
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5 },
                "seven_day": { "used_percentage": 80.1 },
                "model_scoped": [
                    { "display_name": "Fable", "utilization": 12.3, "resets_at": null }
                ]
            },
            "model": { "display_name": "Opus" },
            "effort": { "level": "high" }
        });

        let top_line = [
            "\x1b[38;5;15m/srv/example/project\x1b[0m",
            "\x1b[38;5;63m|",
            "\x1b[38;5;214mfeature\x1b[0m",
        ]
        .join(" ");
        let bottom_line = [
            "\x1b[48;5;24m\x1b[38;5;15m Opus|high \x1b[0m",
            "\x1b[48;5;220m\x1b[38;5;0m ctx 51% \x1b[0m",
            "\x1b[48;5;34m\x1b[38;5;0m 5h 24% \x1b[0m",
            "\x1b[48;5;196m\x1b[38;5;0m 7d 81% \x1b[0m",
            "\x1b[48;5;34m\x1b[38;5;0m 7d Fable 13% \x1b[0m",
        ]
        .join(" ");

        assert_eq!(
            format_status_line(&status),
            Some(format!("{top_line}\n{bottom_line}"))
        );
    }

    #[test]
    fn renders_partial_payloads() {
        // A branch without a dir renders alone, without the separator.
        let only_branch = json!({ "worktree": { "branch": "feature" } });
        assert_eq!(
            format_status_line(&only_branch),
            Some("\x1b[38;5;214mfeature\x1b[0m".to_string())
        );

        let only_bottom = json!({ "context_window": { "used_percentage": 42.0 } });
        assert_eq!(
            format_status_line(&only_bottom),
            Some("\x1b[48;5;34m\x1b[38;5;0m ctx 42% \x1b[0m".to_string())
        );
    }

    #[test]
    fn shows_placeholder_when_dir_has_no_branch() {
        // A directory that exists nowhere fails the git lookup, so the
        // branch side falls back to the placeholder.
        let status = json!({
            "workspace": { "current_dir": "/nonexistent-dir-for-test" }
        });

        assert_eq!(
            format_status_line(&status),
            Some(
                [
                    "\x1b[38;5;15m/nonexistent-dir-for-test\x1b[0m",
                    "\x1b[38;5;63m|",
                    "\x1b[38;5;124m<no git branch>\x1b[0m",
                ]
                .join(" ")
            )
        );
    }

    #[test]
    fn replaces_home_prefix_with_tilde() {
        assert_eq!(
            tildify("/Users/jane/dev/project", Some("/Users/jane")),
            "~/dev/project"
        );
        assert_eq!(tildify("/Users/jane", Some("/Users/jane")), "~");
        assert_eq!(tildify("/Users/jane/dev", Some("/Users/jane/")), "~/dev");
        assert_eq!(
            tildify("/Users/janedoe/dev", Some("/Users/jane")),
            "/Users/janedoe/dev"
        );
        assert_eq!(tildify("/srv/project", None), "/srv/project");
        assert_eq!(
            tildify(r"C:\Users\jane\dev\project", Some(r"C:\Users\jane")),
            r"~\dev\project"
        );
    }

    #[test]
    fn builds_workspace_dir_segment_value() {
        let status = json!({
            "cwd": "/fallback/dir",
            "workspace": { "current_dir": "/Users/jane/dev/some-quite-long-project-name" }
        });

        let dir = workspace_dir_with(&status, Some(PathBuf::from("/Users/jane")));

        assert_eq!(dir, Some("~/dev/some-quite-long-project-name".to_string()));
    }

    #[test]
    fn workspace_dir_falls_back_to_cwd() {
        let status = json!({ "cwd": "/fallback/dir" });

        assert_eq!(
            workspace_dir_with(&status, None),
            Some("/fallback/dir".to_string())
        );
        assert_eq!(workspace_dir_with(&json!({}), None), None);
    }

    #[test]
    fn prefers_payload_model_scoped_over_usage_cache() {
        let status = json!({
            "rate_limits": {
                "model_scoped": [
                    { "display_name": "Fable", "utilization": 12.3 }
                ]
            }
        });

        let limits = model_scoped_limits_with(&status, || panic!("cache should not be read"));

        assert_eq!(limits, vec![("Fable".to_string(), 12.3, None)]);
    }

    #[test]
    fn falls_back_to_usage_cache_for_model_scoped_limits() {
        let status = json!({
            "rate_limits": { "seven_day": { "used_percentage": 2.0 } }
        });
        let cache = json!({
            "cachedUsageUtilization": {
                "utilization": {
                    "limits": [
                        { "kind": "weekly_all", "percent": 2.0 },
                        { "kind": "weekly_scoped", "percent": 4.0 },
                        {
                            "kind": "weekly_scoped",
                            "percent": 4.0,
                            "scope": { "model": { "display_name": "Fable" } }
                        }
                    ]
                }
            }
        });

        let limits = model_scoped_limits_with(&status, || Some(cache.clone()));

        assert_eq!(limits, vec![("Fable".to_string(), 4.0, None)]);
    }

    #[test]
    fn skips_usage_cache_when_payload_has_no_rate_limits() {
        let limits = model_scoped_limits_with(&json!({}), || panic!("cache should not be read"));

        assert_eq!(limits, Vec::new());
    }

    #[test]
    fn returns_none_when_no_segments_can_be_built() {
        assert_eq!(format_status_line(&json!({})), None);
    }

    #[test]
    fn prefers_worktree_branch_over_git_fallback() {
        let status = json!({
            "cwd": "/repo",
            "worktree": { "branch": "worktree-feature" }
        });

        let branch = active_branch_with(&status, |_| Some("main".to_string()));

        assert_eq!(branch, Some("worktree-feature".to_string()));
    }

    #[test]
    fn falls_back_to_cwd_git_branch() {
        let status = json!({
            "cwd": "/repo",
            "workspace": { "current_dir": "/workspace" }
        });

        let branch = active_branch_with(&status, |cwd| {
            assert_eq!(cwd, "/repo");
            Some("main".to_string())
        });

        assert_eq!(branch, Some("main".to_string()));
    }

    #[test]
    fn falls_back_to_workspace_current_dir_when_cwd_git_fails() {
        let status = json!({
            "cwd": "/not-a-repo",
            "workspace": { "current_dir": "/repo" }
        });

        let branch = active_branch_with(&status, |cwd| match cwd {
            "/not-a-repo" => None,
            "/repo" => Some("develop".to_string()),
            _ => panic!("unexpected cwd: {cwd}"),
        });

        assert_eq!(branch, Some("develop".to_string()));
    }

    #[test]
    fn formats_remaining_with_two_largest_units() {
        assert_eq!(format_remaining(-1), "<1m");
        assert_eq!(format_remaining(0), "<1m");
        assert_eq!(format_remaining(60), "<1m");
        assert_eq!(format_remaining(61), "1m");
        assert_eq!(format_remaining(2820), "47m");
        assert_eq!(format_remaining(3600), "1h");
        assert_eq!(format_remaining(15120), "4h12m");
        assert_eq!(format_remaining(518400), "6d");
        assert_eq!(format_remaining(529200), "6d3h");
    }

    #[test]
    fn parses_iso8601_reset_timestamps() {
        // The three spellings all denote the same instant.
        assert_eq!(
            parse_iso8601("2026-07-22T03:59:59.769790+00:00"),
            Some(1784692799)
        );
        assert_eq!(parse_iso8601("2026-07-22T03:59:59Z"), Some(1784692799));
        assert_eq!(parse_iso8601("2026-07-22T05:59:59+02:00"), Some(1784692799));

        assert_eq!(parse_iso8601("not a date"), None);
        assert_eq!(parse_iso8601("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn window_labels_show_time_remaining() {
        let now = 1_000_000_000;
        let status = json!({
            "context_window": { "used_percentage": 10.0 },
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5, "resets_at": now + 15_120 },
                "seven_day": { "used_percentage": 80.1, "resets_at": now + 529_200 },
                "model_scoped": [
                    { "display_name": "Fable", "utilization": 12.3,
                      "resets_at": "2026-07-22T03:59:59.769790+00:00" }
                ]
            }
        });

        let out = format_status_line_with(&status, now, TimeDisplay::Normal)
            .expect("segments should render");

        assert!(out.contains(" 4h12m 24% "), "5h window: {out}");
        assert!(out.contains(" 6d3h 81% "), "7d window: {out}");
        // model_scoped reset is fixed, so pin the countdown against a fixed now.
        let out = format_status_line_with(&status, 1_784_592_000, TimeDisplay::Normal)
            .expect("segments should render");
        assert!(out.contains(" 1d3h Fable 13% "), "per-model window: {out}");
    }

    #[test]
    fn window_labels_stay_static_in_none_mode() {
        let now = 1_000_000_000;
        let status = json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5, "resets_at": now + 15_120 },
                "seven_day": { "used_percentage": 80.1, "resets_at": now + 529_200 },
                "model_scoped": [
                    { "display_name": "Fable", "utilization": 12.3,
                      "resets_at": "2026-07-22T03:59:59.769790+00:00" }
                ]
            }
        });

        let out = format_status_line_with(&status, now, TimeDisplay::None)
            .expect("segments should render");

        assert!(out.contains(" 5h 24% "), "5h window: {out}");
        assert!(out.contains(" 7d 81% "), "7d window: {out}");
        assert!(out.contains(" 7d Fable 13% "), "per-model window: {out}");
    }

    #[test]
    fn window_labels_show_single_unit_in_short_mode() {
        let now = 1_784_592_000;
        let status = json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5, "resets_at": now + 15_120 },
                "seven_day": { "used_percentage": 80.1, "resets_at": now + 529_200 },
                "model_scoped": [
                    { "display_name": "Fable", "utilization": 12.3,
                      "resets_at": "2026-07-22T03:59:59.769790+00:00" }
                ]
            }
        });

        let out = format_status_line_with(&status, now, TimeDisplay::Short)
            .expect("segments should render");

        assert!(out.contains(" 4h 24% "), "5h window: {out}");
        assert!(out.contains(" 6d 81% "), "7d window: {out}");
        assert!(out.contains(" 1d Fable 13% "), "per-model window: {out}");
    }

    #[test]
    fn formats_remaining_short_with_largest_unit() {
        assert_eq!(format_remaining_short(0), "<1m");
        assert_eq!(format_remaining_short(60), "<1m");
        assert_eq!(format_remaining_short(2820), "47m");
        assert_eq!(format_remaining_short(15120), "4h");
        assert_eq!(format_remaining_short(529200), "6d");
    }

    #[test]
    fn parses_time_display_from_env_value() {
        assert_eq!(time_display_from(None), TimeDisplay::Normal);
        assert_eq!(time_display_from(Some("normal")), TimeDisplay::Normal);
        assert_eq!(time_display_from(Some("none")), TimeDisplay::None);
        assert_eq!(time_display_from(Some("short")), TimeDisplay::Short);
        assert_eq!(time_display_from(Some(" NONE ")), TimeDisplay::None);
        assert_eq!(time_display_from(Some("bogus")), TimeDisplay::Normal);
        assert_eq!(time_display_from(Some("")), TimeDisplay::Normal);
    }

    #[test]
    fn window_labels_fall_back_when_reset_time_absent() {
        let status = json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5 },
                "seven_day": { "used_percentage": 80.1 }
            }
        });

        let out = format_status_line_with(&status, 1_000_000_000, TimeDisplay::Normal)
            .expect("segments should render");

        assert!(out.contains(" 5h 24% "), "5h fallback: {out}");
        assert!(out.contains(" 7d 81% "), "7d fallback: {out}");
    }

    #[test]
    fn window_labels_show_expired_windows_as_under_a_minute() {
        let now = 1_000_000_000;
        let status = json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5, "resets_at": now - 100 }
            }
        });

        let out = format_status_line_with(&status, now, TimeDisplay::Normal)
            .expect("segments should render");

        assert!(out.contains(" <1m 24% "), "expired window: {out}");
    }
}
