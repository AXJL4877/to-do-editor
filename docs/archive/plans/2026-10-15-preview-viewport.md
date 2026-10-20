# Preview viewport implementation plan

> Execute inline with superpowers:executing-plans and test-driven-development.

**Goal:** Edit elements outside a fixed output frame using independent viewport zoom and pan.

**Design:** Approved in the conversation: interaction owns a single `preview_viewport.rs` module; preview owns wire transport; native owns presentation; UI exposes controls. Existing projects have no output-resolution field, so the first implementation uses a fixed 16:9 composition frame. Zoom is relative to its aspect fit (10%–400%); Fit restores 100% and centered pan. This does not add project fields, change stored element transforms, or implement exporting.

**Constraints:** Keep the current checkout and its pre-existing uncommitted migration. Do not create a worktree from an older committed tree or commit unrelated files. Use existing `docs/plans`, `native/tests`, `native/build`, and `target/preview_tests` locations. Do not stop the user's running editor.

## File ownership

- New `crates/todo_interaction/src/preview_viewport.rs`: viewport state, fit rectangle, forward/inverse coordinates, anchored zoom and pan.
- Existing interaction input/presentation: translate window input to composition coordinates; retain one-gesture undo, locking, constant screen-size handles; expose view state.
- New `crates/todo_preview/src/viewport.rs`: validated viewport command and navigation wire types. Existing host/protocol: retain navigation alongside pointer edges.
- Existing app bridge/runtime: forward native navigation and synchronize viewport without decoding.
- Existing monitor UI: zoom controls, Fit and output-only switch.
- New `native/src/viewport/` headers: validated viewport geometry/protocol and shared stage shader. Existing presenter/host: retained-frame redraw, canvas background/border, dimmed outside region, output-only clipping, native wheel and middle-button capture.

## Tasks

- [x] 1. Add failing Rust geometry and editor interaction tests, implement viewport module and coordinate conversion. Assert anchored zoom, pan, inverse mapping, invalid inputs, fixed aspect after resize, off-frame editing, undo preservation, locked clips, and gesture stability.
- [x] 2. Add failing protocol tests, implement validated viewport transport and lossless navigation edges; wire app bridge and native input. Assert malformed commands are atomic and navigation remains ordered when queues fill.
- [x] 3. Add failing native math/GPU regressions, implement viewport rendering. Assert transformed source placement, visible off-frame pixels, frame distinction, output-only clipping, retained decode identity, and unchanged legacy protocol behavior.
- [x] 4. Add monitor controls and UI intent tests; update README and ownership documentation; run workspace/native/backend checks and request independent final review.

## Verification

`cargo test --workspace`, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, native build into `native/build/verify-preview`, `python -m unittest discover -s native/tests -v` with that host, and `npm test --prefix backend`.

## Progress and rulings

- User explicitly authorized implementation after the in-chat design; continue without repeating design or execution approvals.
- Keep source changes in the current checkout because the current application lives in untracked migration files; an isolated worktree would omit them.
- No new project schema or resolution-setting UI is part of this request. The fixed output aspect is 16:9 until a future project-output setting supplies it.
- RED/GREEN verified for the missing viewport geometry/protocol/controls/native navigation, late motion after Escape, close/cancel while panning, and unrepresentable projected bounds.
- Final verification: workspace 264 tests pass; native 20 tests pass including WARP pixel assertions; backend 7 tests pass; cargo check, clippy with warnings denied, and formatting of changed Rust files pass.
- Independent final review found one pan lifecycle issue; it was reproduced, fixed using existing gesture cleanup, and regression-tested. No unresolved actionable findings remain.
- The running editor was preserved. Native validation used `native/build/verify-preview`; closing the editor and launching `run.ps1` rebuilds the default host and app from the updated sources.
