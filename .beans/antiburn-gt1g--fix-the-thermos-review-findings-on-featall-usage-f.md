---
# antiburn-gt1g
title: Fix the thermos review findings on feat/all-usage-features
status: todo
type: epic
priority: high
tags:
    - review
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
---

The thermos review (bug/security and code quality) of the fork's 22 commits against upstream main asked for changes: two Windows helper bugs, remote listings that lose their time and size bounds with long history windows, a fork migration that collides with upstream, and maintainability work. Each child bean is one small fix.

Suggested order: Windows copy bug, cost column without a numbered migration (before the next upstream merge), listing bounds, the rest by priority.

Report: `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.
