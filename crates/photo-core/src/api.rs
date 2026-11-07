use crate::{Gallery, Point, Size, View};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub const PROTOCOL_VERSION: u32 = 3;
pub const SOCKET_NAME: &str = "photo-workshop-bigfoot-v3";
pub fn socket_name() -> String {
    std::env::var("AFIO_API_NAME").unwrap_or_else(|_| SOCKET_NAME.into())
}
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
    EnterAlbum,
    NavigatorSnapshot,
    CaptureFrame {
        path: PathBuf,
    },
    ResizeViewport {
        width: u32,
        height: u32,
    },
    SubmitImage {
        options: ImageOptions,
    },
    CancelImage,
    LoadAlbum {
        folder: PathBuf,
    },
    AlbumMove {
        horizontal: i32,
        vertical: i32,
    },
    LoadNested {
        #[serde(default)]
        deeper: bool,
    },
    ShowSearch,
    CloseSearch,
    SearchFiles {
        query: String,
    },
    OpenSearchResult {
        index: usize,
    },
    ConfirmDelete,
    CancelDelete,
    BeginRename,
    CancelRename,
    Rename {
        new_name: String,
    },
    ImportFiles {
        paths: Vec<PathBuf>,
    },
    OpenDropped {
        paths: Vec<PathBuf>,
    },
    ListCommands,
    ShowCommands,
    CloseCommands,
    SearchCommands {
        query: String,
    },
    RunAction {
        id: crate::actions::ActionId,
    },
    VideoPause {
        paused: bool,
    },
    VideoSeek {
        seconds: f64,
    },
    VideoVolume {
        volume: f64,
    },
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
    ToggleSelection {
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VideoState {
    pub path: Option<PathBuf>,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResult {
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperationTiming {
    pub prepare_and_queue_ms: f64,
    pub work_ms: f64,
    pub ui_received_ms: f64,
    pub refresh_ms: f64,
    pub ui_applied_ms: f64,
    pub preview_ready_ms: Option<f64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderState {
    pub preview_ready: bool,
    pub thumbnail_ready: bool,
    pub next_preview_ready: bool,
    pub cached_previews: usize,
    #[serde(default)]
    pub preview_textures: Vec<(PathBuf, u64)>,
    #[serde(default)]
    pub thumbnail_textures: Vec<(PathBuf, u64)>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationState {
    pub id: u64,
    pub action: crate::actions::ActionId,
    pub running: bool,
    pub files: Vec<FileResult>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub timing: Option<OperationTiming>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaletteState {
    pub open: bool,
    pub mode: String,
    pub query: String,
    pub selected: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameState {
    pub source: PathBuf,
    pub new_name: String,
    pub error: Option<String>,
    pub busy: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteState {
    pub paths: Vec<PathBuf>,
    pub count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageOptions {
    pub format: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub preserve_aspect: bool,
    pub target_bytes: Option<u64>,
    pub quality: u8,
    pub strip_metadata: bool,
}
impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            format: "png".into(),
            width: None,
            height: None,
            preserve_aspect: true,
            target_bytes: None,
            quality: 90,
            strip_metadata: false,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageDialogState {
    pub source: PathBuf,
    pub convert: bool,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumState {
    pub folder: PathBuf,
    pub depth: usize,
    pub loaded: usize,
    pub exhausted: bool,
    pub children: usize,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NestedState {
    #[serde(default)]
    pub overview: bool,
    #[serde(default)]
    pub active_album: Option<PathBuf>,
    #[serde(default)]
    pub album_positions: Vec<(PathBuf, usize)>,
    #[serde(default)]
    pub albums: Vec<AlbumState>,
    pub subfolders: Option<usize>,
    pub loading: bool,
    pub expanded: bool,
    pub can_load_more: bool,
    pub can_go_deeper: bool,
    pub depth: usize,
    pub loaded: usize,
    pub visited: usize,
    pub limited: bool,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: PathBuf,
    pub is_folder: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    pub loading: bool,
    pub results: Vec<SearchResult>,
    pub total: u32,
    pub selected: usize,
    pub opening: bool,
    pub open_ms: Option<f64>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    #[serde(default)]
    pub search: Option<SearchState>,
    #[serde(default)]
    pub nested: Option<NestedState>,
    #[serde(default)]
    pub image_dialog: Option<ImageDialogState>,
    pub version: u32,
    pub session_id: Option<String>,
    pub snapshot: Option<Snapshot>,
    pub navigator: Option<NavigatorState>,
    #[serde(default)]
    pub video: Option<VideoState>,
    pub error: Option<ApiError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<crate::actions::ActionInfo>>,
    #[serde(default)]
    pub operation: Option<OperationState>,
    #[serde(default)]
    pub palette: Option<PaletteState>,
    #[serde(default)]
    pub rename: Option<RenameState>,
    #[serde(default)]
    pub delete_confirmation: Option<DeleteState>,
    #[serde(default)]
    pub native_paste_shortcut: bool,
    #[serde(default)]
    pub render: Option<RenderState>,
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
    #[serde(default)]
    pub folder: Option<PathBuf>,
    #[serde(default)]
    pub hovered: Option<PathBuf>,
    pub visible: bool,
    pub focus: Pane,
}
impl Response {
    pub fn ok(gallery: Option<&Gallery>) -> Self {
        Self {
            search: None,
            nested: None,
            image_dialog: None,
            version: PROTOCOL_VERSION,
            session_id: None,
            snapshot: gallery.map(Snapshot::from),
            navigator: None,
            video: None,
            commands: None,
            operation: None,
            palette: None,
            rename: None,
            delete_confirmation: None,
            native_paste_shortcut: false,
            render: None,
            error: None,
        }
    }
    pub fn error(code: ErrorCode, message: impl Into<String>, gallery: Option<&Gallery>) -> Self {
        Self {
            search: None,
            nested: None,
            image_dialog: None,
            version: PROTOCOL_VERSION,
            session_id: None,
            snapshot: gallery.map(Snapshot::from),
            navigator: None,
            video: None,
            commands: None,
            operation: None,
            palette: None,
            rename: None,
            delete_confirmation: None,
            native_paste_shortcut: false,
            render: None,
            error: Some(ApiError {
                code,
                message: message.into(),
            }),
        }
    }
}
