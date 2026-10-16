use crate::TrackKind;
use std::fmt;

/// Machine-readable editing failures. Presentation layers own user-facing text.
#[derive(Debug, Clone, PartialEq)]
pub enum DomainError {
    MediaNotFound(String),
    FolderNotFound(String),
    /// Moving a folder under itself or a descendant.
    FolderCycle(String),
    TrackNotFound(String),
    ClipNotFound(String),
    TrackLocked(String),
    IncompatibleTrack(String),
    Overlap(String),
    SourceBounds(String),
    ClipTooShort(String),
    InvalidTime,
    InvalidDuration,
    InvalidOutputSize,
    InvalidFps,
    InvalidTransform,
    InvalidProject(&'static str),
    DuplicateId(String),
    TrackRequired(TrackKind),
    LinkedEditMismatch,
    RippleConflict,
    MediaInUse(String),
    /// No same-kind neighbor in the requested direction.
    TrackOrder,
    /// The operation does not apply to text (or to media) clips.
    UnsupportedClip(String),
    InvalidText(String),
    InvalidTextStyle(String),
    TextStyleNotFound(String),
}

impl DomainError {
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::MediaNotFound(_) => "media_not_found",
            Self::FolderNotFound(_) => "folder_not_found",
            Self::FolderCycle(_) => "folder_cycle",
            Self::TrackNotFound(_) => "track_not_found",
            Self::ClipNotFound(_) => "clip_not_found",
            Self::TrackLocked(_) => "track_locked",
            Self::IncompatibleTrack(_) => "incompatible_track",
            Self::Overlap(_) => "overlap",
            Self::SourceBounds(_) => "source_bounds",
            Self::ClipTooShort(_) => "clip_too_short",
            Self::InvalidTime => "invalid_time",
            Self::InvalidDuration => "invalid_duration",
            Self::InvalidOutputSize => "invalid_output_size",
            Self::InvalidFps => "invalid_fps",
            Self::InvalidTransform => "invalid_transform",
            Self::InvalidProject(_) => "invalid_project",
            Self::DuplicateId(_) => "duplicate_id",
            Self::TrackRequired(_) => "track_required",
            Self::LinkedEditMismatch => "linked_edit_mismatch",
            Self::RippleConflict => "ripple_conflict",
            Self::MediaInUse(_) => "media_in_use",
            Self::TrackOrder => "track_order",
            Self::UnsupportedClip(_) => "unsupported_clip",
            Self::InvalidText(_) => "invalid_text",
            Self::InvalidTextStyle(_) => "invalid_text_style",
            Self::TextStyleNotFound(_) => "text_style_not_found",
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message_key())
    }
}

impl std::error::Error for DomainError {}
