# Decisions
1. 2026-10-22 — Brief claim "repo already implements trash natively; keep it" is only half true: the `trash` crate does, but on macOS its default shells to `osascript`. Switch to NsFileManager (§5.2). Orchestrator ruling.
2. 2026-10-22 — "Agents" = Claude subagents via Agent tool unless external CLI panes (psmux) are confirmed up. Role mapping in PROFILES.md.
