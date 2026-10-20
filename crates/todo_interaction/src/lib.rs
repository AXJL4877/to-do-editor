//! Input translation and presentation projections. No filesystem, HTTP, or window APIs.
mod input;
mod inspector;
mod presentation;
mod preview_viewport;
mod timeline;
mod transform;
pub use input::{EditorController, EditorFocus, Intent, KeyCode, PreviewPointerKind, TimelineTool};
pub use presentation::ai_settings::{
    normalize_base_url, AiSettingsError, AiSettingsView, AvailableModelView, KeySource,
    ModelCapability, ProxyChoice,
};
pub use presentation::assets::{
    assets_view, font_preview_key, panel_styles, style_preview_key, AssetsInput, AssetsView,
    FontCard, FontInput, MotionCard, MotionOrigin, MotionPose, MotionTarget, StyleCard,
    FONT_SAMPLE, STYLE_SAMPLE,
};
pub use todo_core::MotionKind;
pub use presentation::production::{
    paint_workflow_panel, production_view, production_view_with_document, when_visible, ChoiceView,
    ElementView, ExtensionCardView, ExtensionStatus, FlowNodeView, ParamInput, ParamValue,
    ParamView, ProblemAction, ProblemView, ProductionNodeStatus, ProductionPanelView,
    ProductionView, ScriptView, SegmentView, StageView, VariantView, WorkflowEdgeView,
    WorkflowEntryView, WorkflowMediaFileView, WorkflowNodeView, WorkflowPanelView,
    WorkflowParamFieldView,
};
pub use presentation::{
    fmt_clock, fmt_ruler_tick, fmt_tick, status_text, tick_step, wave_span, AlignGuideView,
    ClipProperties, ClipView, DropPreview, EditorViewModel, ExplorerNode, FolderNodeView,
    LibraryEntryView, LibraryFolderView, LibraryItemView, LibraryMediaView, LibraryViewModel,
    MarqueeView, MotionOption, MotionProperties, MotionSlot, MoveTargetView, PreviewView,
    SourceProperties, StyleOption, TextProperties, Tick,
    TimelineViewModel, TrackView, TransitionMark, Viewport,
};
pub use preview_viewport::{
    PreviewProjection, PreviewViewport, PREVIEW_ZOOM_MAX, PREVIEW_ZOOM_MIN,
};
pub use timeline::{
    clamp_zoom, clip_zone, clips_in_marquee, lane_px, lane_view_width, marquee_selection,
    playhead_x, time_at, timeline_scale, tool_for_key, wheel_zoom, ClipZone, DragGesture, DragKind,
    DragSession, TimelineController, TimelineOutcome, TimelineScale, ZoomHit, EDGE_PX, LABEL_W,
    LANE_MIN_WIDTH, PX_PER_SEC_DEFAULT, PX_PER_SEC_MAX, PX_PER_SEC_MIN, ZOOM_DEFAULT, ZOOM_FACTOR,
    ZOOM_MAX, ZOOM_MIN,
};
pub use todo_application::ProductionEdit;
pub use todo_application::{AssetRequest, CloseChoice, MonitorPane, PeakStrip, PeaksCache};
pub use todo_core::{
    ClipTransform, MediaKind, MediaScaleMode, MotionRef, TextAlign, TextContent, TextIntro, TrackKind,
    Transform, TransformBounds, TrimEdge,
};
pub use todo_production::production::ElementPatch;
