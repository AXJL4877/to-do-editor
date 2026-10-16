# Shared Control Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement inline. Steps use checkbox syntax for tracking.

**Goal:** Implement a transport-independent shared editor API for later internal and external AI callers.

**Architecture:** Serializable contracts live in todo_application/control; Workspace owns execution, request history and import progress. Core owns range placement and linked edits. All effects continue through the existing desktop runtime.

**Tech Stack:** Rust workspace, serde, existing domain/application tests.

**Spec:** `docs/superpowers/specs/2026-10-16-shared-control-design.md`

## Global Constraints

- No model, HTTP/MCP, UI, filesystem or thread implementation in the control layer.
- Protocol version 1; independent session revision; preserve existing manual edit behavior.
- Work in the existing dirty editor checkout; preserve unrelated changes and leave changes uncommitted.
- Sequential copy-based batches, one undo, no mutation on failure or dry run.

## Review Focus

- Partial failures in multi-file imports must finish and retain successful items.
- Manual edit / undo / gesture cancellation must invalidate stale control revisions.
- A replay after intervening edits must never execute twice or repeat effects.
- References to previous steps must resolve actual generated IDs, including linked audio.
- Stale worker events after reopening a project must not affect new state.

## Task 1: Core range placement and linked splitting

Files: core/edit/place_media.rs, command.rs, exports, tests/control_edits.rs.

Interfaces: `EditCommand::PlaceMediaRange { media_id, track_id, time, source_in, source_out }`; `place_media_range(...) -> Result<Project, DomainError>`.

- [x] Add tests for source 3..8 at timeline 10, linked source offsets, short-range placement in an occupied lane, invalid ranges and linked splitting/lock rejection.
- [x] Run `cargo test -p todo_core --test control_edits` and observe missing range API / current linked split failure.
- [x] Factor shared placement implementation around requested source span; add SplitLinked while preserving existing explicit Split behavior.
- [x] Run core tests; preserve existing whole-media placement policy.

## Task 2: Shared query, atomic editing and revisions

Files: application/control/{mod,types,batch,state}.rs, projects/control.rs, editor/commands.rs, editor/mod.rs, application Cargo/dependency boundary, tests/control.rs.

Interfaces: `Workspace::control_context() -> Option<ControlContext>`; `Workspace::control(ControlRequest) -> ControlOutcome`; serializable `ControlRequest`, `ControlCommand`, `ControlEdit`, `ClipRef`, receipts/errors.

- [x] Test query serialization, sequential place/split references, rollback, dryRun, one undo, missing/reused/expired request IDs, stale revision/session, manual mutation revision and active gesture rejection.
- [x] Run the new application test target and observe the missing contract.
- [x] Implement DTOs and domain adapter; run edits against a copy and commit once through the editor; count content changes centrally.
- [x] Implement per-session idempotency and structured errors; clear control state on project lifecycle changes.
- [x] Run application and interaction regression suites.

## Task 3: Library-only imports with task receipts

Files: application action/event, control state, project lifecycle/events, app worker adapter and tests.

Interfaces: `ControlCommand::ImportMedia { paths }`, `GetImportJob { job_id }`, `EditorCommand::RegisterMedia`, `MediaOutput::ImportAborted { message }`.

- [x] Add tests for library-only import, existing UI append, partial errors, duplicate events, stale session callbacks and queue abort completion.
- [x] Run the new tests and observe missing import contracts; reproduce the stopped-worker busy state against the original event behavior.
- [x] Reuse existing import worker/effects, track completion, register library-only results and handle aborted tasks.
- [x] Run application and worker tests.

## Task 4: Documentation, example and verification

Files: docs/architecture/shared-control.md, responsibilities.md, README.md, application/examples/control.rs.

- [x] Document request/result JSON, field/time semantics, replay window, effect execution and explicit version-1 limitations.
- [x] Add a runnable example of place/split/query/undo using the real Workspace.
- [x] Run formatting, workspace tests/check/clippy, the example, and backend regression tests; inspect actual output and record pre-existing global check blockers.
- [x] Review the final change and resolve material issues with reproducing tests.

## Progress

- Baseline: `cargo test --workspace` passed before edits.
- Execution: inline in existing checkout; no commits to avoid absorbing the user's uncommitted migration.
- Review: independent read-only reviewer found stale-status import accounting and permissive nested transform input. Both reproduced RED and fixed GREEN, along with invalid manual import metadata and valid media retention on auto-placement failure.
- Review scope: unrelated migration, backend internals and live UI rendering were not independently reviewed. Shared runtime integration is covered by application/worker tests; no new live UI or transport is part of this task.
- Final verification: 306 Rust workspace tests and 7 backend tests pass; workspace check, production/example Clippy, application/desktop all-target Clippy, new core test Clippy, changed-file rustfmt, and the runnable example pass.
- Existing global blockers: `cargo fmt --all -- --check` reports untouched files (including core/edit/tracks.rs and validate.rs); `cargo clippy --workspace --all-targets -- -D warnings` reports double_ended_iterator_last in core/tests/track_insertion.rs:176. No unrelated full-repository reformat or test-style change was made.
