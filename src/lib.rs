use std::path::PathBuf;
use std::process::Command;

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_FG_BLACK: &str = "\x1b[38;5;0m";
const ANSI_FG_WHITE: &str = "\x1b[38;5;15m";
const ANSI_BG_BLUE: &str = "\x1b[48;5;24m";
const ANSI_BG_PURPLE: &str = "\x1b[48;5;60m";
const ANSI_BG_GREEN: &str = "\x1b[48;5;34m";
const ANSI_BG_YELLOW: &str = "\x1b[48;5;220m";
const ANSI_BG_RED: &str = "\x1b[48;5;196m";

pub fn format_status_line(json: &serde_json::Value) -> Option<String> {
    let segments = status_segments(json);

    if segments.is_empty() {
        None
    } else {
        Some(segments.join(" "))
    }
}

fn status_segments(json: &serde_json::Value) -> Vec<String> {
    let mut segments = Vec::new();

    if let Some(branch) = active_branch(json) {
        segments.push(format_branch_segment(&branch));
    }

    add_percentage_segment(
        &mut segments,
        json,
        "ctx",
        &["context_window", "used_percentage"],
    );
    add_percentage_segment(
        &mut segments,
        json,
        "5h",
        &["rate_limits", "five_hour", "used_percentage"],
    );
    add_percentage_segment(
        &mut segments,
        json,
        "7d",
        &["rate_limits", "seven_day", "used_percentage"],
    );

    for (model_name, value) in model_scoped_limits(json) {
        segments.push(format_percentage_segment(
            &format!("7d {model_name}"),
            value,
        ));
    }

    if let (Some(model_name), Some(effort_level)) = (
        string_at(json, &["model", "display_name"]),
        string_at(json, &["effort", "level"]),
    ) {
        segments.push(format_model_segment(model_name, effort_level));
    }

    segments
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

/// Per-model weekly rate limits (e.g. Fable), reported separately from the
/// all-models `seven_day` bucket.
///
/// Preferred source is `rate_limits.model_scoped` in the status JSON. Claude
/// Code does not emit that field yet, so until it does, fall back to the
/// usage snapshot it caches in `~/.claude.json`.
fn model_scoped_limits(json: &serde_json::Value) -> Vec<(String, f64)> {
    model_scoped_limits_with(json, load_usage_cache)
}

fn model_scoped_limits_with<F>(json: &serde_json::Value, load_usage_cache: F) -> Vec<(String, f64)>
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

fn payload_model_scoped(json: &serde_json::Value) -> Option<Vec<(String, f64)>> {
    let entries = json.get("rate_limits")?.get("model_scoped")?.as_array()?;

    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let name = string_at(entry, &["display_name"])?;
                let value = percentage_at(entry, &["utilization"])?;
                Some((name.to_string(), value))
            })
            .collect(),
    )
}

fn cached_model_scoped(cache: &serde_json::Value) -> Vec<(String, f64)> {
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
            Some((name.to_string(), value))
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
        assert_eq!(
            format_branch_segment("main"),
            "\x1b[48;5;60m\x1b[38;5;15m main \x1b[0m"
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

        assert_eq!(
            format_status_line(&status),
            Some(
                [
                    "\x1b[48;5;60m\x1b[38;5;15m feature \x1b[0m",
                    "\x1b[48;5;220m\x1b[38;5;0m ctx 51% \x1b[0m",
                    "\x1b[48;5;34m\x1b[38;5;0m 5h 24% \x1b[0m",
                    "\x1b[48;5;196m\x1b[38;5;0m 7d 81% \x1b[0m",
                    "\x1b[48;5;34m\x1b[38;5;0m 7d Fable 13% \x1b[0m",
                    "\x1b[48;5;24m\x1b[38;5;15m Opus|high \x1b[0m",
                ]
                .join(" ")
            )
        );
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

        assert_eq!(limits, vec![("Fable".to_string(), 12.3)]);
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

        assert_eq!(limits, vec![("Fable".to_string(), 4.0)]);
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
}
