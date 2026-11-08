//! Afio-owned overrides are applied to private session copies, never standalone Yazi.
use super::*;
use photo_core::settings::{EffectiveYaziBinding, FlavorInfo, Quicklink, YaziBinding, YaziProfile};
use std::collections::BTreeMap;
use std::fs;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value, value};

pub const VERSION: &str = "26.5.6";
pub const DEFAULT_GENERAL: &str = include_str!("../../../assets/yazi-defaults/yazi.toml");
pub const DEFAULT_KEYS: &str = include_str!("../../../assets/yazi-defaults/keymap.toml");
pub const GENERAL: &[(&str, &str, &[&str])] = &[
    ("show_hidden", "Скрытые файлы", &["false", "true"]),
    ("show_symlink", "Показывать ссылки", &["false", "true"]),
    (
        "sort_by",
        "Сортировка",
        &[
            "alphabetical",
            "natural",
            "mtime",
            "btime",
            "extension",
            "size",
            "none",
        ],
    ),
    ("sort_reverse", "Обратный порядок", &["false", "true"]),
    ("sort_dir_first", "Папки первыми", &["false", "true"]),
    ("sort_sensitive", "Учитывать регистр", &["false", "true"]),
    (
        "sort_translit",
        "Транслитерация при сортировке",
        &["false", "true"],
    ),
    (
        "linemode",
        "Подписи файлов",
        &["none", "size", "btime", "mtime", "permissions", "owner"],
    ),
    (
        "scrolloff",
        "Отступ курсора",
        &["0", "1", "2", "3", "5", "8", "10", "15", "20"],
    ),
];
pub const LAYERS: &[&str] = &[
    "mgr", "input", "confirm", "tasks", "spot", "pick", "cmp", "help",
];

pub fn root() -> Result<PathBuf, String> {
    Ok(settings::config_path()?
        .parent()
        .ok_or("No settings directory")?
        .join("yazi"))
}
pub fn load() -> Result<YaziProfile, String> {
    let document = settings::read_config(&settings::config_path()?)?;
    document
        .get("yazi")
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Ok(YaziProfile::default()))
}
fn from_document(document: &serde_json::Value) -> Result<YaziProfile, String> {
    document
        .get("yazi")
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Ok(YaziProfile::default()))
}

/// Staged edits use per-field compare-and-swap. A conflicting field is explicit;
/// unchanged fields always come from disk, including unknown future fields.
fn merge_patch(
    current: &mut serde_json::Value,
    baseline: &serde_json::Value,
    draft: &serde_json::Value,
    field: &str,
) -> Result<(), String> {
    if baseline == draft {
        return Ok(());
    }
    if let (Some(base), Some(next), Some(now)) =
        (baseline.as_array(), draft.as_array(), current.as_array())
    {
        let keyed = |rows: &[serde_json::Value]| {
            rows.iter()
                .all(|row| row.get("id").and_then(serde_json::Value::as_str).is_some())
        };
        if keyed(base) && keyed(next) && keyed(now) {
            let mut rows = now.clone();
            let ids: std::collections::BTreeSet<_> = base
                .iter()
                .chain(next)
                .filter_map(|r| r["id"].as_str())
                .collect();
            for id in ids {
                let old = base
                    .iter()
                    .find(|r| r["id"] == id)
                    .unwrap_or(&serde_json::Value::Null);
                let new = next
                    .iter()
                    .find(|r| r["id"] == id)
                    .unwrap_or(&serde_json::Value::Null);
                if old == new {
                    continue;
                }
                let index = rows.iter().position(|r| r["id"] == id);
                let mut value = index
                    .map(|i| rows[i].clone())
                    .unwrap_or(serde_json::Value::Null);
                merge_patch(&mut value, old, new, &format!("{field}[{id}]"))?;
                if new.is_null() {
                    if let Some(i) = index {
                        rows.remove(i);
                    }
                } else if let Some(i) = index {
                    rows[i] = value;
                } else {
                    rows.push(value);
                }
            }
            *current = serde_json::Value::Array(rows);
            return Ok(());
        }
    }
    if let (Some(base), Some(next)) = (baseline.as_object(), draft.as_object()) {
        if !current.is_object() {
            return Err(format!("Settings conflict at {field}; reload settings."));
        }
        let keys: std::collections::BTreeSet<_> = base.keys().chain(next.keys()).collect();
        for key in keys {
            let old = base.get(key).unwrap_or(&serde_json::Value::Null);
            let new = next.get(key).unwrap_or(&serde_json::Value::Null);
            if old == new {
                continue;
            }
            let mut value = current.get(key).cloned().unwrap_or(serde_json::Value::Null);
            merge_patch(&mut value, old, new, &format!("{field}.{key}"))?;
            if next.contains_key(key) {
                current[key] = value;
            } else {
                current.as_object_mut().unwrap().remove(key);
            }
        }
    } else {
        if current != baseline && current != draft {
            return Err(format!("Settings conflict at {field}; reload settings."));
        }
        *current = draft.clone();
    }
    Ok(())
}

fn merged(
    document: &serde_json::Value,
    baseline: &YaziProfile,
    draft: &YaziProfile,
) -> Result<serde_json::Value, String> {
    let latest = from_document(document)?;
    // Materialize defaults so an absent persisted field compares with its typed baseline.
    let mut current = serde_json::to_value(latest).map_err(|e| e.to_string())?;
    let base = serde_json::to_value(baseline).map_err(|e| e.to_string())?;
    let next = serde_json::to_value(draft).map_err(|e| e.to_string())?;
    merge_patch(&mut current, &base, &next, "yazi")?;
    let mut preserved = document
        .get("yazi")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    fn defaults(raw: &mut serde_json::Value, known: &serde_json::Value) {
        if let (Some(raw), Some(known)) = (raw.as_object_mut(), known.as_object()) {
            for (key, value) in known {
                if let Some(existing) = raw.get_mut(key) {
                    defaults(existing, value);
                } else {
                    raw.insert(key.clone(), value.clone());
                }
            }
        }
    }
    defaults(
        &mut preserved,
        &serde_json::to_value(from_document(document)?).map_err(|e| e.to_string())?,
    );
    // Apply only the actual typed delta to the raw object, retaining unknown keys.
    merge_patch(
        &mut preserved,
        &serde_json::to_value(from_document(document)?).map_err(|e| e.to_string())?,
        &current,
        "yazi",
    )?;
    Ok(preserved)
}

/// Prepare candidate configuration. Commit with save_prepared only after verification.
pub fn prepare_save(
    baseline: &YaziProfile,
    draft: &YaziProfile,
) -> Result<(YaziProfile, YaziProfile), String> {
    prepare_at(&settings::config_path()?, baseline, draft)
}

fn prepare_at(
    path: &Path,
    baseline: &YaziProfile,
    draft: &YaziProfile,
) -> Result<(YaziProfile, YaziProfile), String> {
    let document = settings::read_config(path)?;
    let latest = from_document(&document)?;
    let next =
        serde_json::from_value(merged(&document, baseline, draft)?).map_err(|e| e.to_string())?;
    validate_profile(&next)?;
    Ok((latest, next))
}

pub fn save_prepared(baseline: &YaziProfile, draft: &YaziProfile) -> Result<(), String> {
    save_at(&settings::config_path()?, baseline, draft, true).map(|_| ())
}

fn save_at(
    path: &Path,
    baseline: &YaziProfile,
    draft: &YaziProfile,
    exact: bool,
) -> Result<YaziProfile, String> {
    settings::update_config(path, |document| {
        if exact && from_document(document)? != *baseline {
            return Err(
                "Yazi settings changed in another instance during Apply; retry Apply.".into(),
            );
        }
        let next = merged(document, baseline, draft)?;
        let profile = serde_json::from_value(next.clone()).map_err(|e| e.to_string())?;
        validate_profile(&profile)?;
        document["yazi"] = next;
        Ok(profile)
    })
}

pub fn read(path: &Path) -> Result<DocumentMut, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    text.parse::<DocumentMut>().map_err(|e| e.to_string())
}
pub fn set_general(document: &mut DocumentMut, key: &str, text: &str) -> Result<(), String> {
    let (_, _, options) = GENERAL
        .iter()
        .find(|(id, _, _)| *id == key)
        .ok_or("Unsupported Yazi parameter")?;
    if !options.contains(&text) {
        return Err(format!("Invalid value for {key}"));
    }
    if document.get("mgr").is_none() {
        document["mgr"] = Item::Table(Table::new());
    }
    if !document["mgr"].is_table() {
        return Err("Invalid mgr table".into());
    }
    document["mgr"][key] = match text {
        "true" => value(true),
        "false" => value(false),
        _ if key == "scrolloff" => value(text.parse::<i64>().map_err(|e| e.to_string())?),
        _ => value(text),
    };
    Ok(())
}

type GeneralValues = BTreeMap<String, String>;

pub fn effective_general(
    config: &Path,
    profile: &YaziProfile,
) -> Result<(GeneralValues, GeneralValues), String> {
    let defaults: DocumentMut = DEFAULT_GENERAL
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let current = read(&config.join("yazi.toml"))?;
    let mut values = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for (key, _, _) in GENERAL {
        let own = profile.general.get(*key);
        let user = current
            .get("mgr")
            .and_then(|m| m.get(*key))
            .and_then(Item::as_value);
        let default = defaults["mgr"][key]
            .as_value()
            .ok_or("Missing pinned default")?;
        let show = |v: &Value| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string().trim().to_owned())
        };
        values.insert(
            (*key).into(),
            own.cloned()
                .unwrap_or_else(|| show(user.unwrap_or(default))),
        );
        sources.insert(
            (*key).into(),
            if own.is_some() {
                "Afio override"
            } else if user.is_some() {
                "Yazi session"
            } else {
                "Yazi default"
            }
            .into(),
        );
    }
    Ok((values, sources))
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(v) if v.is_str() => vec![v.as_str().unwrap().into()],
        Some(v) if v.is_array() => v
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}
fn entries(
    document: &DocumentMut,
    layer: &str,
    field: &str,
    source: &str,
) -> Vec<EffectiveYaziBinding> {
    let Some(item) = document.get(layer).and_then(|s| s.get(field)) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    let mut add = |on: Vec<String>, run: Vec<String>, description: String| {
        if !on.is_empty() {
            output.push(EffectiveYaziBinding {
                layer: layer.into(),
                on,
                run,
                description,
                source: source.into(),
            });
        }
    };
    if let Some(array) = item.as_array() {
        for entry in array.iter().filter_map(Value::as_inline_table) {
            add(
                strings(entry.get("on")),
                strings(entry.get("run")),
                entry
                    .get("desc")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
            );
        }
    } else if let Some(array) = item.as_array_of_tables() {
        for entry in array.iter() {
            add(
                strings(entry.get("on").and_then(Item::as_value)),
                strings(entry.get("run").and_then(Item::as_value)),
                entry
                    .get("desc")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .into(),
            );
        }
    }
    output
}
pub fn effective_bindings(config: &Path) -> Result<Vec<EffectiveYaziBinding>, String> {
    let defaults: DocumentMut = DEFAULT_KEYS
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let current = read(&config.join("keymap.toml"))?;
    let mut result = Vec::new();
    for layer in LAYERS {
        let primary = if current.get(layer).and_then(|s| s.get("keymap")).is_some() {
            &current
        } else {
            &defaults
        };
        let prepend = entries(&current, layer, "prepend_keymap", "Yazi prepend");
        let primary = entries(
            primary,
            layer,
            "keymap",
            if std::ptr::eq(primary, &defaults) {
                "Yazi default"
            } else {
                "Yazi keymap"
            },
        );
        // Pinned Yazi merges priority tiers by their first two keys, but keeps
        // longer distinct sequences inside a tier. Do not truncate the action
        // catalogue when three-key chords share their prefix.
        let signature = |b: &EffectiveYaziBinding| b.on.iter().take(2).cloned().collect::<Vec<_>>();
        let prepend_keys: HashSet<_> = prepend.iter().map(signature).collect();
        let primary_keys: HashSet<_> = primary.iter().map(signature).collect();
        let mut combined = prepend;
        combined.extend(
            primary
                .into_iter()
                .filter(|b| !prepend_keys.contains(&signature(b))),
        );
        combined.extend(
            entries(&current, layer, "append_keymap", "Yazi append")
                .into_iter()
                .filter(|b| !primary_keys.contains(&signature(b))),
        );
        let mut seen = HashSet::new();
        for binding in combined {
            if seen.insert(binding.on.clone()) && binding.run != ["noop"] {
                result.push(binding);
            }
        }
    }
    Ok(result)
}
pub fn validate_binding(binding: &YaziBinding) -> Result<(), String> {
    if binding.id.is_empty()
        || binding.id.len() > 100
        || !LAYERS.contains(&binding.layer.as_str())
        || binding.description.len() > 200
    {
        return Err("Invalid binding ID, description or Yazi layer".into());
    }
    if binding.on.is_empty()
        || binding.on.len() > 8
        || binding
            .on
            .iter()
            .any(|k| k.is_empty() || k.len() > 40 || k.chars().any(char::is_control))
    {
        return Err("Yazi bindings require one to eight valid keys".into());
    }
    if binding.on.iter().any(|k| {
        [
            "<Tab>",
            "<S-Tab>",
            "<BackTab>",
            "<C-k>",
            "<C-K>",
            "<C-S-k>",
            "<S-C-k>",
            "<C-,>",
        ]
        .iter()
        .any(|reserved| k.eq_ignore_ascii_case(reserved))
    }) {
        return Err("Tab, Ctrl+K, Ctrl+Shift+K and Ctrl+, belong to Afio; Shift+Tab opens native file information".into());
    }
    // Native commands are supported; arbitrary Lua/shell authoring stays in the original config.
    static NATIVE_COMMANDS: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    let commands = NATIVE_COMMANDS.get_or_init(|| {
        let defaults: DocumentMut = DEFAULT_KEYS.parse().expect("Pinned keymap is valid TOML");
        let mut names: HashSet<String> = LAYERS
            .iter()
            .flat_map(|layer| entries(&defaults, layer, "keymap", "default"))
            .flat_map(|entry| entry.run)
            .filter_map(|command| command.split_whitespace().next().map(str::to_owned))
            .filter(|command| !matches!(command.as_str(), "shell" | "plugin"))
            .collect();
        names.extend(["noop".to_owned(), "reveal".to_owned()]);
        names
    });
    if binding.run.is_empty()
        || binding.run.len() > 8
        || binding.run.iter().any(|s| {
            s.len() > 4096
                || s.chars().any(char::is_control)
                || !commands.contains(s.split_whitespace().next().unwrap_or(""))
        })
    {
        return Err(
            "Use supported native Yazi commands; shell and Lua remain in the source config".into(),
        );
    }
    Ok(())
}
pub fn quicklink_binding(link: &Quicklink) -> Result<YaziBinding, String> {
    if link.key.chars().count() != 1
        || link
            .key
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        || link.name.is_empty()
        || link.name.len() > 200
        || link.id.is_empty()
    {
        return Err("Quicklink needs a name, ID and one key after g".into());
    }
    if !link.path.is_absolute() || (link.enabled && !link.path.is_dir()) {
        return Err("Choose an existing absolute folder".into());
    }
    let path = friendly_path(&link.path).replace('\\', "/");
    if path.chars().any(|c| c.is_control() || c == '"') {
        return Err("Unsupported characters in folder path".into());
    }
    Ok(YaziBinding {
        id: link.id.clone(),
        layer: "mgr".into(),
        on: vec!["g".into(), link.key.clone()],
        run: vec![format!("cd --raw \"{path}\"")],
        description: link.name.clone(),
        enabled: link.enabled,
    })
}

/// Validate the complete result after merging independently staged edits. Row-level
/// validation cannot detect a key or ID introduced by another open instance.
pub fn validate_profile(profile: &YaziProfile) -> Result<(), String> {
    for (namespace, ids) in [
        (
            "quicklinks",
            profile
                .quicklinks
                .iter()
                .map(|link| &link.id)
                .collect::<Vec<_>>(),
        ),
        (
            "bindings",
            profile
                .bindings
                .iter()
                .map(|binding| &binding.id)
                .collect::<Vec<_>>(),
        ),
    ] {
        let mut seen = HashSet::new();
        for id in ids {
            if id.is_empty() || !seen.insert(id) {
                return Err(format!(
                    "Yazi profile conflict: duplicate or invalid {namespace} ID {id:?}. Reload settings before applying."
                ));
            }
        }
    }
    let mut general = DocumentMut::new();
    for (key, value) in &profile.general {
        set_general(&mut general, key, value)?;
    }
    if profile
        .icons
        .as_deref()
        .is_some_and(|icons| !["inherit", "minimal"].contains(&icons))
    {
        return Err("Unknown icon set".into());
    }
    let mut active = Vec::new();
    for binding in &profile.bindings {
        if binding.enabled {
            validate_binding(binding)?;
            active.push(binding.clone());
        }
    }
    for link in profile
        .quicklinks
        .iter()
        .filter(|link| link.enabled && !link.deleted)
    {
        let binding = quicklink_binding(link)?;
        validate_binding(&binding)?;
        active.push(binding);
    }
    for (index, binding) in active.iter().enumerate() {
        if let Some(other) = active[..index].iter().find(|other| {
            other.layer == binding.layer
                && (other.on.starts_with(&binding.on) || binding.on.starts_with(&other.on))
        }) {
            return Err(format!(
                "Yazi profile conflict: {} ({}) and {} ({}) overlap in {}. Change the draft or reload settings before applying.",
                other.id,
                other.on.join(" → "),
                binding.id,
                binding.on.join(" → "),
                binding.layer
            ));
        }
    }
    Ok(())
}

pub fn apply(config: &Path, profile: &YaziProfile) -> Result<(), String> {
    validate_profile(profile)?;
    let mut general = read(&config.join("yazi.toml"))?;
    for (key, text) in &profile.general {
        set_general(&mut general, key, text)?;
    }
    fs::write(config.join("yazi.toml"), general.to_string()).map_err(|e| e.to_string())?;
    let mut keymap = read(&config.join("keymap.toml"))?;
    let mut additions = profile
        .bindings
        .iter()
        .filter(|b| b.enabled)
        .cloned()
        .collect::<Vec<_>>();
    for link in profile
        .quicklinks
        .iter()
        .filter(|l| l.enabled && !l.deleted)
    {
        additions.push(quicklink_binding(link)?);
    }
    for binding in additions.iter().rev() {
        validate_binding(binding)?;
        if keymap.get(&binding.layer).is_none() {
            keymap[&binding.layer] = Item::Table(Table::new());
        }
        if keymap[&binding.layer].as_table_like().is_none() {
            return Err(format!("Invalid {} keymap table", binding.layer));
        }
        let slot = &mut keymap[&binding.layer]["prepend_keymap"];
        let mut array = slot.as_array().cloned().unwrap_or_default();
        if let Some(tables) = slot.as_array_of_tables() {
            for t in tables {
                array.push(t.clone().into_inline_table());
            }
        }
        let mut row = InlineTable::new();
        row.insert("on", Value::Array(Array::from_iter(binding.on.clone())));
        row.insert("run", Value::Array(Array::from_iter(binding.run.clone())));
        row.insert("desc", Value::from(binding.description.clone()));
        array.insert(0, Value::InlineTable(row));
        *slot = value(array);
    }
    fs::write(config.join("keymap.toml"), keymap.to_string()).map_err(|e| e.to_string())?;
    let mut theme = read(&config.join("theme.toml"))?;
    for (key, flavor) in [
        ("dark", &profile.dark_flavor),
        ("light", &profile.light_flavor),
    ] {
        if let Some(name) = flavor {
            if !name.is_empty() {
                validate_flavor(&config.join("flavors").join(format!("{name}.yazi")))?;
            }
            if theme.get("flavor").is_none() {
                theme["flavor"] = Item::Table(Table::new());
            }
            if theme["flavor"].as_table_like().is_none() {
                return Err("Invalid flavor table".into());
            }
            theme["flavor"][key] = value(name);
        }
    }
    if profile.icons.as_deref() == Some("minimal") {
        let mut globs = theme
            .get("icon")
            .and_then(|icon| icon.get("prepend_globs"))
            .and_then(Item::as_array)
            .cloned()
            .unwrap_or_default();
        for (url, text) in [("*/", "▸"), ("*", "·")] {
            let mut icon = InlineTable::new();
            icon.insert("url", url.into());
            icon.insert("text", text.into());
            globs.push(icon);
        }
        if theme.get("icon").is_none() {
            theme["icon"] = Item::Table(Table::new());
        }
        if theme["icon"].as_table_like().is_none() {
            return Err("Invalid icon table".into());
        }
        theme["icon"]["prepend_globs"] = value(globs);
    } else if profile.icons.as_deref().is_some_and(|v| v != "inherit") {
        return Err("Unknown icon set".into());
    }
    fs::write(config.join("theme.toml"), theme.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn validate_flavor(path: &Path) -> Result<FlavorInfo, String> {
    let name = path
        .file_name()
        .ok_or("Invalid flavor path")?
        .to_string_lossy()
        .trim_end_matches(".yazi")
        .to_owned();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Invalid flavor name".into());
    }
    let theme = read(&path.join("flavor.toml"))?;
    if theme.is_empty() || !path.join("tmtheme.xml").is_file() || !path.join("LICENSE").is_file() {
        return Err("Flavor needs flavor.toml, tmtheme.xml and LICENSE".into());
    }
    if theme.get("manager").is_some() {
        return Err("Flavor targets an older Yazi schema".into());
    }
    let provenance: serde_json::Value = fs::read(path.join("afio-source.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    Ok(FlavorInfo {
        name,
        source: provenance["source"]
            .as_str()
            .unwrap_or("Installed Yazi flavor")
            .into(),
        revision: provenance["revision"].as_str().unwrap_or("unknown").into(),
        license: fs::read_to_string(path.join("LICENSE"))
            .map_err(|e| e.to_string())?
            .chars()
            .take(240)
            .collect(),
        compatible: true,
        preview: path
            .join("preview.png")
            .is_file()
            .then(|| path.join("preview.png")),
        problem: None,
    })
}
pub fn flavors(config: &Path) -> Vec<FlavorInfo> {
    fs::read_dir(config.join("flavors"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".yazi"))
        .map(|entry| {
            validate_flavor(&entry.path()).unwrap_or_else(|error| FlavorInfo {
                name: entry
                    .file_name()
                    .to_string_lossy()
                    .trim_end_matches(".yazi")
                    .into(),
                source: "Installed Yazi flavor".into(),
                revision: "unknown".into(),
                license: String::new(),
                compatible: false,
                preview: None,
                problem: Some(error),
            })
        })
        .collect()
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn mode_persistence_is_independent_and_old_profiles_default_dark() {
        use photo_core::settings::YaziThemeMode;
        let path =
            std::env::temp_dir().join(format!("afio-mode-settings-{}.json", std::process::id()));
        fs::write(
            &path,
            r#"{"version":1,"gallery_scale":1.25,"yazi":{"general":{},"future":"keep"}}"#,
        )
        .unwrap();
        let old = from_document(&settings::read_config(&path).unwrap()).unwrap();
        assert_eq!(old.theme_mode, YaziThemeMode::Dark);
        let mut light = old.clone();
        light.theme_mode = YaziThemeMode::Light;
        save_at(&path, &old, &light, true).unwrap();
        let document = settings::read_config(&path).unwrap();
        assert_eq!(document["gallery_scale"], 1.25);
        assert_eq!(document["yazi"]["theme_mode"], "light");
        assert_eq!(document["yazi"]["future"], "keep");
        assert_eq!(
            from_document(&document).unwrap().theme_mode,
            YaziThemeMode::Light
        );
        save_at(&path, &light, &old, true).unwrap();
        assert_eq!(
            from_document(&settings::read_config(&path).unwrap())
                .unwrap()
                .theme_mode,
            YaziThemeMode::Dark
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn native_eight_key_sequences_and_tier_merging_keep_distinct_long_chords() {
        let root = std::env::temp_dir().join(format!("afio-long-chords-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("keymap.toml"), "[mgr]\nprepend_keymap=[{on=[\"x\",\"y\",\"a\"],run=\"arrow 1\"},{on=[\"x\",\"y\",\"b\"],run=\"arrow -1\"}]\nkeymap=[{on=[\"x\",\"y\",\"c\"],run=\"arrow 2\"}]\n").unwrap();
        let effective = effective_bindings(&root).unwrap();
        assert!(effective.iter().any(|b| b.on == ["x", "y", "a"]));
        assert!(effective.iter().any(|b| b.on == ["x", "y", "b"]));
        assert!(!effective.iter().any(|b| b.on == ["x", "y", "c"]));
        let mut binding = YaziBinding {
            id: "long".into(),
            layer: "mgr".into(),
            on: vec!["x".into(); 8],
            run: vec!["arrow 1".into()],
            description: String::new(),
            enabled: true,
        };
        validate_binding(&binding).unwrap();
        binding.on.push("y".into());
        assert!(validate_binding(&binding).is_err());
        for key in ["<C-S-k>", "<S-C-k>", "<S-Tab>", "<BackTab>"] {
            binding.on = vec![key.into()];
            assert!(
                validate_binding(&binding).is_err(),
                "reserved {key} must not bypass recorder validation"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_quicklink_drafts_validate_merged_key_ownership_before_prepare_or_commit() {
        let root = std::env::temp_dir().join(format!(
            "afio-yazi-semantic-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        let path = root.join("settings.json");
        let baseline = YaziProfile::default();
        let link = |id: &str, key: &str| Quicklink {
            id: id.into(),
            name: id.into(),
            key: key.into(),
            path: root.join(id),
            enabled: true,
            deleted: false,
        };
        let mut a = baseline.clone();
        a.quicklinks.push(link("a", "1"));
        let mut b = baseline.clone();
        b.quicklinks.push(link("b", "1"));
        save_at(&path, &baseline, &a, true).unwrap();
        let original = fs::read(&path).unwrap();
        let local_draft = b.clone();
        assert!(
            prepare_at(&path, &baseline, &b)
                .unwrap_err()
                .contains("overlap")
        );
        assert_eq!(b, local_draft);
        assert!(
            save_at(&path, &baseline, &b, false)
                .unwrap_err()
                .contains("overlap")
        );
        let mut invalid = a.clone();
        invalid.quicklinks.extend(b.quicklinks.clone());
        assert!(
            save_at(&path, &a, &invalid, true)
                .unwrap_err()
                .contains("overlap")
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        b.quicklinks[0].key = "2".into();
        let (latest, next) = prepare_at(&path, &baseline, &b).unwrap();
        assert_eq!(latest, a);
        assert_eq!(next.quicklinks.len(), 2);
        save_at(&path, &latest, &next, true).unwrap();
        assert_eq!(
            from_document(&settings::read_config(&path).unwrap()).unwrap(),
            next
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_profile_rejects_cross_kind_prefix_conflicts_and_duplicate_ids() {
        let mut profile = YaziProfile::default();
        profile.quicklinks.push(Quicklink {
            id: "link".into(),
            name: "Link".into(),
            key: "1".into(),
            path: std::env::temp_dir(),
            enabled: true,
            deleted: false,
        });
        profile.bindings.push(YaziBinding {
            id: "binding".into(),
            layer: "mgr".into(),
            on: vec!["g".into()],
            run: vec!["noop".into()],
            description: "prefix".into(),
            enabled: true,
        });
        assert!(validate_profile(&profile).unwrap_err().contains("overlap"));
        profile.bindings[0].enabled = false;
        validate_profile(&profile).unwrap();
        profile.bindings[0].enabled = true;
        assert!(validate_profile(&profile).unwrap_err().contains("overlap"));
        profile.bindings[0].on = vec!["g".into(), "2".into()];
        validate_profile(&profile).unwrap();
        profile.bindings[0].id = "link".into();
        validate_profile(&profile).unwrap(); // IDs belong to independent persisted lists.
        profile.bindings.push(profile.bindings[0].clone());
        assert!(
            validate_profile(&profile)
                .unwrap_err()
                .contains("duplicate")
        );
        profile.bindings.clear();
        profile.quicklinks.push(profile.quicklinks[0].clone());
        assert!(
            validate_profile(&profile)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn stale_drafts_merge_fields_and_reject_same_field_conflicts() {
        let root = std::env::temp_dir().join(format!(
            "afio-yazi-cas-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        fs::write(
            &path,
            r#"{"version":1,"unknown":true,"yazi":{"future":{"keep":42}}}"#,
        )
        .unwrap();
        let baseline = YaziProfile::default();
        let mut a = baseline.clone();
        a.general.insert("scrolloff".into(), "5".into());
        let mut b = baseline.clone();
        b.icons = Some("minimal".into());
        save_at(&path, &baseline, &a, false).unwrap();
        let merged = save_at(&path, &baseline, &b, false).unwrap();
        assert_eq!(merged.general["scrolloff"], "5");
        assert_eq!(merged.icons.as_deref(), Some("minimal"));
        let document = settings::read_config(&path).unwrap();
        assert_eq!(document["yazi"]["future"]["keep"], 42);
        assert_eq!(document["unknown"], true);
        assert!(path.with_extension("json.previous").is_file());
        a.general.insert("scrolloff".into(), "8".into());
        assert!(
            save_at(&path, &baseline, &a, false)
                .unwrap_err()
                .contains("conflict")
        );
        assert!(
            save_at(&path, &baseline, &b, true)
                .unwrap_err()
                .contains("during Apply")
        );
        assert_eq!(settings::read_config(&path).unwrap(), document);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn row_patch_preserves_other_rows_and_unknown_row_fields() {
        let baseline = serde_json::json!([{"id":"a","enabled":true}]);
        let draft = serde_json::json!([{"id":"a","enabled":false}]);
        let mut current =
            serde_json::json!([{"id":"a","enabled":true,"future":42},{"id":"b","enabled":true}]);
        merge_patch(&mut current, &baseline, &draft, "bindings").unwrap();
        assert_eq!(current[0]["future"], 42);
        assert_eq!(current[0]["enabled"], false);
        assert_eq!(current[1]["id"], "b");
    }
}
