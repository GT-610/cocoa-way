---
name: cocoa-way-diagnose
description: Diagnose Cocoa-Way compositor, Apple Container, Docker, OrbStack, waypipe, display, clipboard, performance, and rootless/desktop failures. Use when reproducing a bug, triaging logs, checking a new machine, or drafting a concise redacted GitHub issue.
---

# Diagnose Cocoa-Way

Collect evidence before changing code. Distinguish host compositor, transport, runtime, guest image, and guest application failures instead of treating every warning as the root cause.

## Workflow

1. Call `cocoa_way_environment`, `cocoa_way_features`, and `cocoa_way_status`.
2. Call `cocoa_way_sessions` and `cocoa_way_displays`. If one session is implicated, call `cocoa_way_logs` for it.
3. Prefer `cocoa_way_diagnostics` for a single redacted snapshot. If MCP is unavailable, run `cocoa-wayctl --json diagnostics [SESSION]` against the local control socket.
4. Reproduce once with the smallest suitable guest application. Use desktop presentation for niri/Hyprland and rootless only for regular xdg-shell applications.
5. Classify the first actionable failure:
   - host: Cocoa-Way crash, Metal/GLES error, focus/input, resize, or display lifecycle
   - transport: waypipe/socket/relay disconnect, latency, clipboard, or audio
   - runtime: Apple Container, Docker, OrbStack, image, volume, or machine lifecycle
   - guest: missing executable/library/socket, unsupported image, compositor configuration
6. Confirm whether the process really failed. Locale watcher, missing icon, Xwayland integration, and dmabuf fallback warnings can be non-fatal when the surface still renders.
7. Call `cocoa_way_issue_draft` with factual steps, expected result, and actual result. Review and trim the generated Markdown before submission.

## Safety

- Keep diagnosis read-only unless the user explicitly authorizes a change.
- Never delete images, containers, volumes, machines, or profiles as a diagnostic shortcut.
- Never post an issue or upload logs automatically.
- Preserve the Metal rendering path; do not force software rendering to hide a transport or buffer bug.
- Redact home paths, account names, tokens, registry credentials, SSH hosts, and unrelated process output.
- Include exact Cocoa-Way, Apple Container, waypipe, macOS, architecture, presentation, and transport versions when relevant.

## Issue Quality

Produce one issue per independently reproducible defect. Include the shortest reproduction, whether desktop/rootless and default/dedicated display were used, the first actionable error, and whether the problem survives a clean Cocoa-Way restart. Do not pad reports with harmless warning floods or speculative fixes.
