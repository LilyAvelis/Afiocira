use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Gallery, Point, Size, View};

pub const PROTOCOL_VERSION: u32 = 3;
pub const SOCKET_NAME: &str = "photo-workshop-bigfoot-v3";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pane {
    Gallery,
    Navigator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Status,
    ListOrder,
    Open {
        path: PathBuf,
    },
    PreviewFolder {
        path: PathBuf,
    },
    Select {
        index: usize,
    },
    SelectPath {
        path: PathBuf,
    },
    ExtendSelection {
        index: usize,
    },
    SetMarked {
        paths: Vec<PathBuf>,
    },
    SetOrder {
        paths: Vec<PathBuf>,
    },
    OpenSelected,
    Next,
    Previous,
    Back,
    Refresh,
    SetNavigatorVisible {
        visible: bool,
    },
    FocusPane {
        pane: Pane,
    },
    ZoomAt {
        multiplier: f32,
        anchor: Point,
        viewport: Size,
        fitted: Size,
    },
    PanBy {
        delta: Point,
        viewport: Size,
        fitted: Size,
    },
    ResetZoom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub folder: PathBuf,
    pub count: usize,
    pub view: View,
    pub selected_index: Option<usize>,
    pub selected_path: Option<PathBuf>,
    pub grid_anchor: usize,
    pub cursor_path: Option<PathBuf>,
    pub marked_paths: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered_paths: Option<Vec<PathBuf>>,
    pub zoom_factor_of_fit: f32,
}

impl From<&Gallery> for Snapshot {
    fn from(gallery: &Gallery) -> Self {
        Self {
            folder: gallery.folder().to_path_buf(),
            count: gallery.photos().len(),
            view: gallery.view(),
            selected_index: gallery.selected_index(),
            selected_path: gallery.selected_photo().map(|photo| photo.path.clone()),
            grid_anchor: gallery.grid_anchor(),
            cursor_path: gallery.cursor_photo().map(|photo| photo.path.clone()),
            marked_paths: gallery.marked_paths().iter().cloned().collect(),
            ordered_paths: None,
            zoom_factor_of_fit: gallery.zoom().factor,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub session_id: Option<String>,
    pub snapshot: Option<Snapshot>,
    pub navigator: Option<NavigatorState>,
    pub error: Option<ApiError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidCommand,
    InvalidPath,
    InvalidSelection,
    InvalidState,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigatorState {
    pub visible: bool,
    pub focus: Pane,
}

impl Response {
    pub fn ok(gallery: Option<&Gallery>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            session_id: None,
            snapshot: gallery.map(Snapshot::from),
            navigator: None,
            error: None,
        }
    }

    pub fn error(code: ErrorCode, message: impl Into<String>, gallery: Option<&Gallery>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            session_id: None,
            snapshot: gallery.map(Snapshot::from),
            navigator: None,
            error: Some(ApiError {
                code,
                message: message.into(),
            }),
        }
    }
}
