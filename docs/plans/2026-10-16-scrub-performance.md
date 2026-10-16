# Scrub Performance Implementation Plan

> **For agentic workers:** Use systematic debugging and test-driven-development per task; independent Rust session, native scheduler and media-proxy work may proceed in parallel. Root integrates and verifies. The user explicitly requested implementation after supplying the unfinished-work report.

**Goal:** Smooth continuous and reverse timeline scrubbing without blocking video on text, cloning the project per pointer event, or waiting serially for every video layer.

**Architecture:** Keep Rust application state and the MF/D3D11 preview host. Preserve the current progressive-frame changes, add bounded per-stream native scheduling, and generate reusable all-I-frame H.264 editing proxies in a separate Rust background worker. Original media remains authoritative and stored unchanged. Test exact and approximate updates separately.

**Tech Stack:** Rust workspace, C++17 Media Foundation/D3D11, packaged FFmpeg/FFprobe, Python native protocol tests.

**Spec:** User handoff in this chat, plus read-only source investigation on 2026-10-16.

## Constraints and review focus

- Preserve all existing uncommitted changes and user data; no Node dependency.
- Keep source-change cancellation and final exact frame correctness; approximate frames must not masquerade as exact frames.
- Text fallback is only for the same clip/layout; missing text must not freeze video or resurrect another caption.
- Proxies are disposable caches, with bounded work, atomic publication, original-path/timing identity, failure fallback and stale-project isolation.
- Shared GPU resources, decoder ownership and global memory use must stay bounded under multi-layer decode.
- Tests use independent native build directories while the old app is running; package only after verification, never kill the user's app.

## Task 1: Rust hot paths

Files: `todo_application/src/playback/`, `editor/preview.rs`, `text_rasters/`, application integration tests, small core seek helper if needed.

- [ ] Add red tests for allocation-preserving scrub, video progress while text is pending, compatible text fallback, incompatible caption/layout invalidation, and late raster completion.
- [ ] Replace whole-project seek cloning with playhead-only state updates, retaining clamping/history/dirty semantics.
- [ ] Publish available video immediately, with a compatible ready text raster when possible.
- [ ] Run focused application/core tests, then integration suite.

## Task 2: Native scheduling and regression coverage

Files: `native/src/video.*`, focused scheduler helper files as needed, decoder progress boundaries, `native/tests/`.

- [ ] Add red regressions for approximate progress, large jumps, stale-source cancellation and precise final multi-layer output.
- [ ] Replace serial per-layer waits with a bounded scheduler, independent decoder ownership and safe retained GPU frames; publish progressive layers without a slow-layer barrier.
- [ ] Bound aggregate cache use instead of multiplying an unconditional per-decoder budget.
- [ ] Build to an isolated directory, run native regressions and reproducible forward/reverse/jump benchmarks.

## Task 3: Editing proxy pipeline and desktop integration

Files: new `todo_media/src/proxies/`, media exports/tests, new app proxy adapter and dedicated worker, preview bridge/runtime wiring.

- [ ] Test real FFmpeg output/cache reuse/invalidation and unsupported/failure handling before implementing proxy generation.
- [ ] Expose `MediaService::get_edit_proxy`, use a versioned identity cache and atomic output, retain source timing/SAR and validate generated media.
- [ ] Generate asynchronously on import/open/use with a separate bounded queue; original media is immediately usable while pending.
- [ ] Route visual preview through ready proxies, maintain original dimensions/transforms/audio/reference identity, recover to original on failure. Test bridge mapping and late completion/project switches.
- [ ] Benchmark proxy-backed scrubbing separately from original mode; report fps, update gaps, target error and final exact latency rather than fps alone.

## Task 4: Verification and package

- [ ] Independent review and resolve important findings.
- [ ] Full Rust tests/check/production Clippy and changed-target strict Clippy; focused formatting.
- [ ] Full native tests with the exact tested binary; record its path/hash and benchmark methodology.
- [ ] Rebuild package; if the user's running executable prevents replacement, publish a sibling package with matching Rust/native binaries and report its exact path.
- [ ] Record evidence, limitations and file responsibilities in architecture/report docs.
