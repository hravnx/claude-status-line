# claude-status-line

A small Rust status-line formatter for Claude Code.

It reads Claude Code status JSON from stdin and prints compact ANSI-colored
segments across two rows. The first row shows the workspace directory (with
the home directory shown as `~`) and the branch; the second row shows:

- model and effort level
- context window usage
- 5-hour and 7-day rate limit usage
- per-model weekly rate limit usage (e.g. Fable), which is tracked
  separately from the all-models 7-day limit

Percentage segments are rounded up and colored by usage:

- green: up to 50%
- yellow: up to 80%
- red: above 80%

## Usage

Build and install the binary somewhere on your `PATH`:

```bash
cargo install --path .
```

Then configure Claude Code to use it as a status line in
`~/.claude/settings.json`:

```json
{
  "statusLine": {
    "type": "command",
    "command": "claude-status-line"
  }
}
```

Claude Code passes status JSON on stdin. You can test the formatter locally
with the included fixture:

```bash
claude-status-line < tests/fixtures/schema.json
```

Example output includes ANSI styling and segments like:

![Example status line](docs/status-line.svg)

```text
~/dev/my-project  worktree-my-feature
Opus|high  ctx 32%  5h 81%  7d 65%  7d Fable 4%
```

### Per-model rate limits

Some models (currently Fable) have their own weekly rate limit that is not
included in the generic `seven_day` bucket. The formatter reads these from
`rate_limits.model_scoped` in the status JSON when present. Claude Code does
not emit that field yet, so as a fallback the formatter reads the usage
snapshot Claude Code caches in `~/.claude.json`
(`cachedUsageUtilization.utilization.limits`). The fallback is only used when
the status JSON contains `rate_limits`, and it disappears automatically once
Claude Code starts sending `model_scoped` itself. Note that the cache key is
an undocumented Claude Code internal and may change between releases; if it
does, the segment is silently omitted.

## Development

```bash
cargo test
cargo run --quiet < tests/fixtures/schema.json
```

## License

MIT
