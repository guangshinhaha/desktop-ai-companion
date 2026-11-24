# 0004: The shell is Tauri v2

Status: accepted, 24 Nov 2026. Refs #23, TD section 3.1, TD section 10.

## Context

The shell is what the officer sees: the menu-bar icon, the one-input canvas, the replay progress view, the approval card and the review screen. It runs on managed laptops that may be slow, and is idle most of the day. It must work on macOS now and Windows later. One person builds it.

## Decision

Use Tauri v2: a Rust core with a web UI. The shell runs the executor as a sidecar process. The executor stays native (decision 0002) and draws the recording indicator itself, so the shell starts in Phase 1.

## Alternatives

| Option | For | Against |
|---|---|---|
| Electron | One UI codebase for both systems, large ecosystem. | Ships a full Chromium with every app, so idle memory is high. Too heavy for the target laptop. |
| Native UIs (SwiftUI, then WinUI) | Best macOS fit. | Two UIs to build and maintain, for one person. |
| Tauri v2 | One UI codebase. Uses the system web view, so idle memory is a fraction of Electron's. Rust core fits the sidecar and encrypted local store. | Younger ecosystem. Web view differs between macOS and Windows. |

## Consequences

- The idle budget in TD section 10 applies to the shell. #25 measures it on a SEED Mac and reports the number.
- If the measured idle memory misses the budget, this note is revisited before more UI is built.
- The UI stays plain web, so a later move to another host costs the Rust core only.
- Windows web view differences are tested when Windows work starts, not before.
