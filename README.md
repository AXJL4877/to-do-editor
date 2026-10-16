# TO DO

Rust-native timeline editor (eframe/egui). The repo root **is** the Cargo workspace.

```
to_do/
  Cargo.toml          workspace
  crates/
    todo_core         project / clip model
    todo_application  project/editor use cases, history, playback, task state
    todo_interaction  input, gestures, snapping, view models
    todo_ui           egui widgets
    todo_media        local import / probe / thumbnails / waveforms / text raster cache / tool execution
    todo_text         styled text (花字) rasterizer: fonts, layout, layer effects
    todo_storage      local project documents and atomic persistence
    todo_preview      preview_host.exe protocol
    todo_app          eframe host + wiring
  dependencies/       packaged native media tools (FFmpeg / FFprobe) and text fonts
  tests/support/      Rust subprocess fixtures shared by protocol/media tests
  native/
    src/              preview host, video worker, decoder, presenter, audio
    tests/            native preview protocol regressions
    build/bin/        generated native binaries / object files
  data/               project JSON + media
  run.ps1             build preview if needed + run todo_app
  build.ps1           package the native app and media tools into dist/to_do
```

## Run

```powershell
# once
native\build.cmd

# every time
.\run.ps1
```

Or:

```powershell
cargo run -p todo_app
```

## Check

```powershell
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --lib --bins --examples -- -D warnings
cargo clippy -p todo_application -p todo_interaction -p todo_ui --all-targets -- -D warnings
python -m unittest discover -s native/tests -v
```

Native tests require Windows, a built preview host, and FFmpeg (bundled under
`dependencies/ffmpeg/bin/` or available on PATH). Test videos are generated temporarily
under `target/preview_tests/`. Set `TODO_TEST_MEDIA` to test an existing long-GOP
video of at least nine seconds instead.

Video decoding runs on one COM-initialized worker. Its latest request includes
both the media path and source time, replaces pending work, and cancels obsolete
decoding between samples. Nearby replacements reuse unfinished decoding so
continuous scrubbing still produces frames. The window thread presents only
current completed frames, retaining the MF sample until the frame is replaced.
Layer updates keep the last visible stack until its complete replacement is
ready. Placement, scale, and opacity changes reuse the retained GPU frames;
obsolete completion messages cannot clear the canvas. An explicit empty layer
stack clears it immediately.
Recent decoded frames, including keyframe preroll, are cached in independent
GPU textures for backward scrubbing. The cache retains at most 128 MiB and 120
frames, and is cleared when the source closes or the output media type changes.
Seeking outside the cached range still requires decoding from a keyframe.
Releasing the playhead submits the final precise position again. The native
`video-frame <serial> <target> <timestamp> <duration>` stdout diagnostic reports
accepted decoding completions; it does not measure physical display latency.

Optional env: `TODO_REPO_ROOT`, `TODO_PREVIEW_HOST`, `TODO_FFMPEG`, `TODO_FFPROBE`, `TODO_FONTS_DIR`.

Media processing runs directly in Rust workers. No Node/npm installation, local
HTTP server, or backend port is required for builds, tests or startup. FFmpeg and
FFprobe are native executables invoked only for media jobs and reaped when done.
Their locations are resolved at runtime, so Rust compilation does not need
FFmpeg development libraries. See [native tools](dependencies/README.md).

Package a standalone directory with `./build.ps1` (release), or
`./build.ps1 -Configuration debug`. The package contains `todo_app.exe`,
`preview_host.exe`, media tools and their notices. Run the packaged executable
directly; its `data/` and `cache/` directories belong to that package directory.

## Editor workspace

Drag the dividers beside the media library and the right inspector to change
their widths. Drag above the timeline to change its height. Double-click a
divider to restore its default. Pane preferences and the restored window size
are saved between sessions; temporary small or minimized windows do not replace
the preferred layout. Selecting a clip only updates the inspector and keeps the program
canvas geometry unchanged. Play/pause is below the program preview.

Use **工程设置** in the top toolbar for output resolution, frame rate, media
placement scaling and fixed workspace duration. These controls live in a modal;
the native preview and editor shortcuts are blocked while it is open.

The media library uses an indented explorer tree with expand/collapse arrows,
folder icons and video/audio/image icons. Click a row to select it and press
Delete while the library is focused to remove that item. Deleting a bin moves
its contents to its parent; media used by timeline clips remains protected.
Source files are retained.

Import with Ctrl+I or drop files into the editor. Drag library media into a blank
timeline range, or use the row context menu to append. Double-click opens the
source monitor. V selects, C enables the razor,
Ctrl+K cuts at the playhead, Delete removes selected clips, and Shift+Delete
closes the gap. Ctrl+Z undoes an edit. Empty editable tracks are cleaned up after
clip deletion, retaining at least one video and one audio track.

The selection and razor tools use icons with V/C tooltips. Use the plus on the
left of a track header to insert an empty track of the same kind directly below
that row. Linked selection selects existing linked clips together; switching it
off lets you select individually and does not unlink the clips.

While dragging library media, moving left of the timeline start keeps the
placement preview at zero. Release alongside the visible destination track to
place it there. Locked tracks and incompatible track types reject placement. Occupied ranges
can spill onto a new compatible track; Escape cancels the drag.

Drag the left/right clip handles to trim; hold Alt to stretch time instead.
The selected clip inspector provides duration, normalized X/Y, scale, and reset.
Drag directly inside the native preview to reposition the visible picture.
Select the visible clip to show its transform box. Drag an edge handle to resize
one dimension, or a corner to scale proportionally around the opposite corner.
The inspector also exposes independent width/height scale and opacity. Position
uses canvas-width/height offsets; all scale controls and opacity display percent.
Transform and selection-overlay updates redraw the retained GPU frame without
reopening or seeking the video. Escape cancels a gesture; locked clips reject it.
The program monitor has an independent editing viewport over a fixed 16:9
output frame. Use the mouse wheel over the picture to zoom around the pointer,
or drag with the middle button to pan. The header offers zoom out/in, the
current fit-relative percentage, **适应** (Fit, 100%, centered), and **仅输出**
(output-only view). Editing mode dims the area outside the output frame while
keeping off-frame elements selectable and draggable. Output-only view masks
that area and hides selection handles. Viewport navigation is session-local,
does not alter element transforms or project history, and reuses decoded frames.
Project output resolution is configured in **工程设置** and determines the
composition aspect independently of the monitor panel shape. Exporting is not
implemented yet.
Moves, edge edits, and library placement snap to neighboring boundaries and the
playhead; one gesture creates one undo entry. Escape cancels the active gesture.
Alt/Ctrl+wheel zooms the timeline around the time under the mouse. Ordinary
scrolling keeps its separate horizontal/vertical scroll behavior.
Snapped boundaries retain their exact time, including media durations between
frames. Moving into an occupied range reuses a free track of the required kind
or creates one; linked video and audio keep their timing together. Locked
source or explicitly targeted tracks reject the whole move.
The configured workspace duration is independent from placed content. A 300-second
workspace stays 300 seconds when a longer media file is imported; the full clip
remains editable beyond that boundary. Playback stops at the configured end, while
the timeline scrolls far enough to show overflow and marks the workspace boundary.

Timeline movement rules and module layout: [时间轴编辑](docs/architecture/timeline-editing.md).

## Styled text (花字)

**文字** in the toolbar adds styled text at the playhead on a text track (always
above video). Edit its words, style, size, spacing, alignment and intro (fade or
per-character) in the inspector. Text renders on a dedicated worker into cached
transparent PNGs; the preview keeps the previous picture until a text image is
ready. In the preview, text is selected and transformed by its own text box.

Fonts are looked up by file name in `TODO_FONTS_DIR`, `dependencies/fonts/` and
the Windows font folder. Data model, units, JSON and rendering pipeline:
[花字层标准](docs/architecture/text-layer.md). Render the reference frame with
`cargo run -p todo_text --example reference --release`.

Project documents are stored locally by Rust. Returning waits for successful
save; closing offers Save / Discard / Cancel. Project deletion moves its folder
to `data/.trash`. Local media imports are streamed into temporary files, probed,
and published only on success. Failed imports leave no partial media files.

Folder ownership and extension rules: [项目文件夹分工](docs/architecture/responsibilities.md).

## Shared editor control

`todo_application::control` exposes transport-independent context queries,
library imports with job receipts, source-range placement, sequential atomic edit
batches, dry runs, and undo/redo. Built-in assistants and external AI adapters can
share this API. Hosts execute the returned `AppUpdate` through their existing
effect handler; no model connection or HTTP/MCP server is started.

```powershell
cargo run -p todo_application --example control
```

Protocol, integration rules and JSON examples: [共用编辑能力层](docs/architecture/shared-control.md).

Native media layout and runtime behavior: [Rust 媒体处理](docs/architecture/native-media.md).
