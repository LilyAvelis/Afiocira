//! Serializable settings contracts. No GUI dependency.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct YaziProfile {
    pub theme_mode: YaziThemeMode,
    pub general: BTreeMap<String, String>,
    pub quicklinks: Vec<Quicklink>,
    pub bindings: Vec<YaziBinding>,
    pub dark_flavor: Option<String>,
    pub light_flavor: Option<String>,
    pub icons: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YaziThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quicklink {
    pub id: String,
    pub name: String,
    pub key: String,
    pub path: PathBuf,
    pub enabled: bool,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YaziBinding {
    pub id: String,
    pub layer: String,
    pub on: Vec<String>,
    pub run: Vec<String>,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum YaziRequest {
    ApplyThemeMode {
        mode: YaziThemeMode,
    },
    ApplyAppearance {
        dark: Option<String>,
        light: Option<String>,
        icons: Option<String>,
    },
    TestQuicklink {
        id: String,
    },
    PickFolder,
    Inspect,
    SetGeneral {
        key: String,
        value: Option<String>,
    },
    SaveQuicklink {
        link: Quicklink,
        #[serde(default)]
        replace: bool,
    },
    DeleteQuicklink {
        id: String,
    },
    RestoreQuicklink {
        id: String,
    },
    SaveBinding {
        binding: YaziBinding,
        #[serde(default)]
        replace: bool,
    },
    DeleteBinding {
        id: String,
    },
    SetAppearance {
        dark: Option<String>,
        light: Option<String>,
        icons: Option<String>,
    },
    InstallFlavor {
        source: String,
    },
    Apply,
    Discard,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct YaziSettingsSnapshot {
    pub version: String,
    pub config_directory: Option<PathBuf>,
    pub profile: YaziProfile,
    pub effective_general: BTreeMap<String, String>,
    pub general_sources: BTreeMap<String, String>,
    pub bindings: Vec<EffectiveYaziBinding>,
    pub inherited_bindings: Vec<EffectiveYaziBinding>,
    pub flavors: Vec<FlavorInfo>,
    pub dirty: bool,
    pub busy: bool,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveYaziBinding {
    pub layer: String,
    pub on: Vec<String>,
    pub run: Vec<String>,
    pub description: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlavorInfo {
    pub name: String,
    pub source: String,
    pub revision: String,
    pub license: String,
    pub compatible: bool,
    pub preview: Option<PathBuf>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutInfo {
    pub action: crate::actions::ActionId,
    pub context: String,
    pub label: String,
    pub binding: Option<String>,
    pub default_binding: Option<String>,
    pub rebindable: bool,
}

#[cfg(windows)]
pub fn choose_folder() -> Result<Option<PathBuf>, String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog,
        IFileOpenDialog, SIGDN_FILESYSPATH,
    };
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        struct ComGuard;
        impl Drop for ComGuard {
            fn drop(&mut self) {
                unsafe { CoUninitialize() }
            }
        }
        let _guard = ComGuard;
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| e.to_string())?;
        dialog
            .SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR)
            .map_err(|e| e.to_string())?;
        if let Err(error) = dialog.Show(None) {
            if error.code().0 as u32 == 0x800704c7 {
                return Ok(None);
            }
            return Err(error.to_string());
        }
        let path = dialog
            .GetResult()
            .map_err(|e| e.to_string())?
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| e.to_string())?;
        let text = path.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(path.0.cast()));
        Ok(Some(PathBuf::from(text?)))
    }
}
