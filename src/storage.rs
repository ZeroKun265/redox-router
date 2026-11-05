//! Persistent layout: node positions (keyed by node name, since ids change
//! every session), pan and zoom. Stored as JSON in the user's config dir,
//! e.g. ~/.config/RedoxRouter/layout.json on Linux.

use crate::rules::{GroupEntry, Rule, RuleGroup};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn one() -> f32 {
    1.0
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

pub fn save(layout: &LayoutFile) -> Result<(), String> {
    save_json("layout.json", layout)
}

/// The shared rule library (used by every setup).
pub fn load_library() -> Vec<RuleGroup> {
    load_json("rule_library.json").unwrap_or_default()
}

pub fn save_library(library: &[RuleGroup]) -> Result<(), String> {
    save_json("rule_library.json", &library)
}
