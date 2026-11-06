//! Persistent layout: node positions (keyed by node name, since ids change
//! every session), pan and zoom. Stored as JSON in the user's config dir,
//! e.g. ~/.config/RedoxRouter/layout.json on Linux.

use crate::links::SavedLink;
use crate::setups::SetupList;
use crate::rules::{GroupEntry, Rule, RuleGroup};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn one() -> f32 {
    1.0
}

fn yes_bool() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutFile {
    #[serde(default)]
    pub pan: (f32, f32),
    #[serde(default = "one")]
    pub zoom: f32,
    /// node.name -> world position of the node's top-left corner.
    #[serde(default)]
    pub nodes: BTreeMap<String, (f32, f32)>,
    /// node.name of manually hidden nodes.
    #[serde(default)]
    pub hidden: BTreeSet<String>,
    /// node.name -> manually set width.
    #[serde(default)]
    pub widths: BTreeMap<String, f32>,
    /// node.name -> user nickname shown on the node's tab.
    #[serde(default)]
    pub nicknames: BTreeMap<String, String>,
    /// Old format (flat rule list); migrated into a group on load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Rule groups (own or library references), applied in order.
    #[serde(default)]
    pub groups: Vec<GroupEntry>,
    /// Saved connections (by node/port name), re-created when both ends exist.
    #[serde(default)]
    pub links: Vec<SavedLink>,
    /// Mode for hiding sink nodes: ShowAll, HideRedox, or HideAll.
    #[serde(default)]
    pub sink_hide_mode: SinkHideMode,
    /// Audio bleed / auto-connection prevention mode.
    #[serde(default)]
    pub auto_connect_prevention: AutoConnectPrevention,
    /// Deprecated legacy field; migrated to sink_hide_mode if true.
    #[serde(default, skip_serializing)]
    pub hide_all_sinks: bool,
    /// Comments and colored blocks.
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Curved or zigzag cables.
    #[serde(default)]
    pub link_style: crate::layout::LinkStyle,
    /// Whether external (non-Redox) links are shown on this setup's canvas.
    #[serde(default)]
    pub show_external: bool,
    /// Virtual sinks and virtual microphones belonging to this setup.
    #[serde(default)]
    pub virtual_devices: Vec<SavedSink>,
    /// Whether dragging nodes, blocks, comments, and waypoints snaps to the grid.
    #[serde(default)]
    pub snap_to_grid: bool,
    /// Whether the corner minimap is shown on this setup.
    #[serde(default = "yes_bool")]
    pub show_minimap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteKind {
    /// Sticky text comment (drawn above nodes).
    Text,
    /// Translucent colored block with a title (drawn behind everything).
    Block,
}

/// A text comment or colored block on the canvas (world coordinates).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub kind: NoteKind,
    pub pos: (f32, f32),
    /// Width and height; a comment's height follows its text.
    pub size: (f32, f32),
    pub text: String,
    pub color: [u8; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VirtualDeviceKind {
    /// Virtual speaker (`Audio/Sink`).
    #[default]
    Sink,
    /// Virtual microphone (`Audio/Source/Virtual`).
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSink {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub kind: VirtualDeviceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AutoConnectPrevention {
    #[default]
    Off,
    ManagedNodes,
    AllNodes,
}

impl AutoConnectPrevention {
    pub const ALL: [AutoConnectPrevention; 3] = [
        AutoConnectPrevention::Off,
        AutoConnectPrevention::ManagedNodes,
        AutoConnectPrevention::AllNodes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AutoConnectPrevention::Off => "Allow all",
            AutoConnectPrevention::ManagedNodes => "Block for managed nodes",
            AutoConnectPrevention::AllNodes => "Block for all new apps",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SinkHideMode {
    #[default]
    ShowAll,
    HideRedox,
    HideAll,
}

impl SinkHideMode {
    pub const ALL: [SinkHideMode; 3] = [
        SinkHideMode::ShowAll,
        SinkHideMode::HideRedox,
        SinkHideMode::HideAll,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SinkHideMode::ShowAll => "Show all",
            SinkHideMode::HideRedox => "Hide Redox only",
            SinkHideMode::HideAll => "Hide all",
        }
    }
}

impl Default for LayoutFile {
    fn default() -> Self {
        Self {
            pan: (20.0, 20.0),
            zoom: 1.0,
            nodes: BTreeMap::new(),
            hidden: BTreeSet::new(),
            widths: BTreeMap::new(),
            nicknames: BTreeMap::new(),
            rules: Vec::new(),
            groups: Vec::new(),
            links: Vec::new(),
            sink_hide_mode: SinkHideMode::ShowAll,
            auto_connect_prevention: AutoConnectPrevention::Off,
            hide_all_sinks: false,
            notes: Vec::new(),
            link_style: Default::default(),
            show_external: false,
            virtual_devices: Vec::new(),
            snap_to_grid: false,
            show_minimap: true,
        }
    }
}

fn path(file: &str) -> Option<PathBuf> {
    ProjectDirs::from("", "ZeroKun265", "RedoxRouter").map(|d| d.config_dir().join(file))
}

fn load_json<T: serde::de::DeserializeOwned>(file: &str) -> Option<T> {
    let text = std::fs::read_to_string(path(file)?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Save atomically (write a temp file, then rename over the old one).
fn save_json<T: Serialize>(file: &str, value: &T) -> Result<(), String> {
    let path = path(file).ok_or("no config directory available")?;
    let dir = path.parent().ok_or("bad config path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Load the saved layout; `None` if there is none or it cannot be read.
pub fn load() -> Option<LayoutFile> {
    load_json("layout.json")
}

/// All setups. The first run migrates the old single `layout.json` into "Main",
/// and migrates legacy `sinks.json` into the active setup's `virtual_devices`.
pub fn load_setups() -> SetupList<LayoutFile> {
    let mut l: SetupList<LayoutFile> = load_json("setups.json")
        .unwrap_or_else(|| SetupList::new("Main", load().unwrap_or_default()));
    l.fix(LayoutFile::default);
    if l.items.iter().all(|s| s.data.virtual_devices.is_empty()) {
        let legacy = load_sinks();
        if !legacy.is_empty() {
            l.items[l.active].data.virtual_devices = legacy;
        }
    }
    l
}

pub fn save_setups(setups: &SetupList<LayoutFile>) -> Result<(), String> {
    save_json("setups.json", setups)
}

/// The shared rule library (used by every setup).
pub fn load_library() -> Vec<RuleGroup> {
    load_json("rule_library.json").unwrap_or_default()
}

pub fn save_library(library: &[RuleGroup]) -> Result<(), String> {
    save_json("rule_library.json", &library)
}

/// Persistent Redox-created virtual sinks and microphones.
pub fn load_sinks() -> Vec<SavedSink> {
    load_json("sinks.json").unwrap_or_default()
}

pub fn save_sinks(sinks: &[SavedSink]) -> Result<(), String> {
    save_json("sinks.json", &sinks)
}

/// Directory for exported setup `.json` files (`~/.config/RedoxRouter/exports`).
pub fn exports_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "ZeroKun265", "RedoxRouter").map(|d| d.config_dir().join("exports"))
}

/// Sanitize a setup name into a safe filename stem.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "setup".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Default file path for exporting a setup with the given name.
pub fn default_export_path(setup_name: &str) -> PathBuf {
    let file = format!("{}.json", sanitize_filename(setup_name));
    exports_dir()
        .map(|d| d.join(&file))
        .unwrap_or_else(|| PathBuf::from(file))
}

/// List existing `.json` files in the exports directory.
pub fn list_export_files() -> Vec<PathBuf> {
    let Some(dir) = exports_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

/// Expand leading `~/` to the user's home directory if present.
pub fn expand_tilde(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(trimmed)
}

/// Build a portable `Setup<LayoutFile>` where any referenced library rule groups
/// are embedded as unique local rule groups, and serialize it to pretty JSON.
pub fn export_setup_json(
    setup: &crate::setups::Setup<LayoutFile>,
    library: &[RuleGroup],
) -> Result<String, String> {
    let mut portable = setup.clone();
    if !portable.data.rules.is_empty() {
        let group = RuleGroup {
            name: "Rules".to_string(),
            enabled: true,
            rules: std::mem::take(&mut portable.data.rules),
        };
        portable.data.groups.insert(0, GroupEntry::Local(group));
    }
    portable.data.groups = crate::rules::standalone_groups(&portable.data.groups, library);
    serde_json::to_string_pretty(&portable).map_err(|e| e.to_string())
}

/// Write a portable setup JSON to the specified file path.
pub fn export_setup_to_path(
    path: &std::path::Path,
    setup: &crate::setups::Setup<LayoutFile>,
    library: &[RuleGroup],
) -> Result<(), String> {
    let json = export_setup_json(setup, library)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Parse an imported setup from JSON. Accepts either a full `Setup<LayoutFile>`
/// (`{ "name": "...", "data": { ... } }`) or a bare `LayoutFile` (using `fallback_name`).
/// Any library references in the imported file are converted to standalone local groups.
pub fn parse_imported_setup(
    json: &str,
    fallback_name: &str,
    library: &[RuleGroup],
) -> Result<crate::setups::Setup<LayoutFile>, String> {
    let mut setup: crate::setups::Setup<LayoutFile> =
        match serde_json::from_str::<crate::setups::Setup<LayoutFile>>(json) {
            Ok(s) => s,
            Err(_) => {
                let data: LayoutFile =
                    serde_json::from_str(json).map_err(|e| format!("invalid setup JSON: {e}"))?;
                let name = if fallback_name.trim().is_empty() {
                    "Imported".to_string()
                } else {
                    fallback_name.trim().to_string()
                };
                crate::setups::Setup { name, data }
            }
        };
    if setup.name.trim().is_empty() {
        setup.name = "Imported".to_string();
    }
    if !setup.data.rules.is_empty() {
        let group = RuleGroup {
            name: "Rules".to_string(),
            enabled: true,
            rules: std::mem::take(&mut setup.data.rules),
        };
        setup.data.groups.insert(0, GroupEntry::Local(group));
    }
    setup.data.groups = crate::rules::standalone_groups(&setup.data.groups, library);
    Ok(setup)
}

/// Read and parse a setup from a file path on disk.
pub fn import_setup_from_path(
    path: &std::path::Path,
    library: &[RuleGroup],
) -> Result<crate::setups::Setup<LayoutFile>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let fallback = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported");
    parse_imported_setup(&text, fallback, library)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Action, Field, Mode};

    #[test]
    fn export_and_import_setup_makes_library_groups_standalone() {
        let lib_rule = Rule {
            field: Field::Title,
            mode: Mode::Pattern,
            pattern: "HideMe".to_string(),
            action: Action::Hide,
            enabled: true,
        };
        let library = vec![RuleGroup {
            name: "Shared Filter".to_string(),
            enabled: true,
            rules: vec![lib_rule.clone()],
        }];
        let mut data = LayoutFile::default();
        data.groups.push(GroupEntry::Library {
            name: "Shared Filter".to_string(),
            enabled: true,
        });
        let setup = crate::setups::Setup {
            name: "Streaming".to_string(),
            data,
        };

        let json = export_setup_json(&setup, &library).unwrap();
        // Importing on another machine with an empty library still preserves the rules!
        let imported = parse_imported_setup(&json, "Fallback", &[]).unwrap();
        assert_eq!(imported.name, "Streaming");
        assert_eq!(imported.data.groups.len(), 1);
        match &imported.data.groups[0] {
            GroupEntry::Local(g) => {
                assert_eq!(g.name, "Shared Filter");
                assert!(g.enabled);
                assert_eq!(g.rules, vec![lib_rule]);
            }
            GroupEntry::Library { .. } => panic!("expected standalone Local group"),
        }
    }

    #[test]
    fn saved_sink_defaults_to_sink_kind_when_omitted() {
        let old_json = r#"{"name":"Virtual_Sink_1","description":"Virtual Sink"}"#;
        let parsed: SavedSink = serde_json::from_str(old_json).unwrap();
        assert_eq!(parsed.kind, VirtualDeviceKind::Sink);
    }
}
