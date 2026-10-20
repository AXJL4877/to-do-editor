# Native Media Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans inline. Track steps with checkboxes.

**Goal:** Remove Node from application startup, media operations, builds and tests.

**Architecture:** Rust MediaService owns local media I/O, cache and FFmpeg calls. ProjectStore owns fenced project directories. Existing application queues/control contracts remain the execution path.

**Tech Stack:** Rust/std, existing serde/uuid/thiserror dependencies, packaged FFmpeg tools.

**Spec:** `docs/superpowers/specs/2026-10-16-native-media-design.md`

## Global Constraints

- User permits native FFmpeg/FFprobe executables; no Node/npm/HTTP backend.
- Retain source files and existing data/project documents.
- Work in current uncommitted migration checkout, without commits or branch changes.
- All generated test data stays in target; tool binaries and licenses in dependencies.

## Review Focus

- Probe/copy failures must leave no published or temporary imported file.
- Relative paths, project names and reparse points must not escape project boundaries.
- Stereo PCM fragments and long media must not be buffered in full.
- Replaced sources and different widths/bins must invalidate cached results.
- Dead tools and worker failures must finish cleanly without leaving processes or Node requirements.

## Task 1: Direct Rust media service

Files: todo_media/src/{service,error}, imports/, metadata/, thumbnails/, waveforms/, cache/, tools/; todo_storage projects; native media integration tests.

Interfaces: MediaService::new(root), MediaTools::discover(root), MediaService::with_tools(root,tools), import_file(project,path), probe_media(stored), get_thumb(stored,width), get_peaks(stored,bins); MediaMetadata, Waveform; ProjectStore::media_directory(name).

- [x] Write real-media and failure tests, observe missing service API, replace obsolete HTTP-contract tests.
- [x] Implement fenced streamed import and metadata parsing, bounded native process execution, thumbnails, PCM buckets and cache.
- [x] Pass media and storage tests with actual FFmpeg-generated fixtures.

## Task 2: App/runtime integration and Node removal

Files: app adapters/workers, paths, runtime; preview host tests/support; dependencies; backend; run.ps1/build.ps1; Cargo manifests and boundary tests.

- [x] Migrate worker tests to actual local files and retain partial-failure/completion assertions.
- [x] Wire MediaService, remove URL configuration and ureq dependency, replace all Node test subprocesses with Rust fixtures.
- [x] Relocate FFmpeg safely, retire backend to ignored target/retired-node-backend, remove obsolete JS, add Node-free build/start scripts and package resources.
- [x] Run workspace tests and test/build with Node absent from PATH.

## Task 3: Documentation and final verification

- [x] Update current architecture, README, tool layout and packaging instructions; preserve historical reports.
- [x] Verify format, check, scoped Clippy, native preview regressions, example and debug package.
- [x] Independent code review, reproduce/fix material findings, record evidence and limitations.

## Progress

- Baseline workspace tests passed.
- FFmpeg external tools explicitly authorized by user; Node runtime and tests to be removed.
- Delivered native media service, worker integration and resource packaging; 312 Rust tests and 20 native preview tests pass.
- Independent review findings fixed: anchor runtime cache ignore rule and regenerate corrupt JPEG caches.
- Packaging uses target/package to avoid Windows file locks on a running development executable.
- Evidence and remaining boundaries: docs/reports/2026-10-16-native-media.md.
