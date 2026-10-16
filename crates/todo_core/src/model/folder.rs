use serde::{Deserialize, Serialize};

/// Virtual media-library folder (Premiere-style bin). Not a disk directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaFolder {
    pub id: String,
    pub name: String,
    /// Parent folder id; `None` means the library root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}
