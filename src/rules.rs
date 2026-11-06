//! Filter rules: match nodes by text (the shown title, technical name,
//! description or media class), then hide them or color their header.
//! Matching is case-insensitive. A pattern without `*` or `?` matches anywhere
//! in the text; with wildcards it must match the whole text (`*` = any run of
//! characters, `?` = any one character). An empty pattern matches nothing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Field {
    Any,
    /// The title shown on the node (its description, or its name).
    Title,
    /// The user's nickname for the node (empty if none; `?*` = has a nickname).
    Nickname,
    /// The technical node.name.
    Name,
    /// The technical node.name, whole text, no wildcards.
    NameExact,
    Description,
    MediaClass,
}

impl Field {
    pub const ALL: [Field; 7] = [
        Field::Title,
        Field::Nickname,
        Field::Name,
        Field::NameExact,
        Field::Description,
        Field::MediaClass,
        Field::Any,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Any => "Any field",
            Field::Title => "Title (as shown)",
            Field::Nickname => "Nickname",
            Field::Name => "Node name (technical)",
            Field::NameExact => "Node name (exact)",
            Field::Description => "Description",
            Field::MediaClass => "Media class",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Hide,
    Show,
    Color([u8; 3]),
}

/// How a rule compares its pattern with the node's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Mode {
    /// Plain text matches anywhere; with `*` / `?` the whole text must match.
    #[default]
    Pattern,
    /// The whole text equals the pattern (case-insensitive, no wildcards).
    Exact,
    /// The text is empty or only spaces; no pattern needed.
    Empty,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Pattern, Mode::Exact, Mode::Empty];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Pattern => "Contains / wildcards",
            Mode::Exact => "Exact match",
            Mode::Empty => "Is empty",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub field: Field,
    #[serde(default)]
    pub mode: Mode,
    pub pattern: String,
    pub action: Action,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Case-insensitive wildcard match of the whole text.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Text a node can be matched on.
pub struct NodeText<'a> {
    pub title: &'a str,
    pub nickname: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub media_class: &'a str,
}

/// Media classes match exactly (case-insensitive) unless the pattern has wildcards.
fn class_matches(pattern: &str, class: &str) -> bool {
    if pattern.contains(['*', '?']) {
        glob_match(pattern, class)
    } else {
        pattern.eq_ignore_ascii_case(class)
    }
}

fn text_matches(pattern: &str, text: &str) -> bool {
    if pattern.contains(['*', '?']) {
        glob_match(pattern, text)
    } else {
        text.to_lowercase().contains(&pattern.to_lowercase())
    }
}

impl Rule {
    pub fn new() -> Rule {
        Rule {
            field: Field::Title,
            mode: Mode::Pattern,
            pattern: String::new(),
            action: Action::Hide,
            enabled: true,
        }
    }

    /// The text(s) of the node this rule's field refers to.
    fn texts<'a>(&self, t: &NodeText<'a>) -> Vec<&'a str> {
        match self.field {
            Field::Any => vec![t.title, t.nickname, t.name, t.description, t.media_class],
            Field::Title => vec![t.title],
            Field::Nickname => vec![t.nickname],
            Field::Name | Field::NameExact => vec![t.name],
            Field::Description => vec![t.description],
            Field::MediaClass => vec![t.media_class],
        }
    }

    pub fn matches(&self, t: &NodeText) -> bool {
        match self.mode {
            // Blank means empty or only spaces; with "any field", all of them.
            Mode::Empty => return self.texts(t).iter().all(|s| s.trim().is_empty()),
            Mode::Exact => {
                let want = self.pattern.to_lowercase();
                return !self.pattern.is_empty()
                    && self.texts(t).iter().any(|s| s.to_lowercase() == want);
            }
            Mode::Pattern => {}
        }
        if self.pattern.is_empty() {
            return false;
        }
        let hit = |s: &str| text_matches(&self.pattern, s);
        match self.field {
            Field::Any => {
                hit(t.title)
                    || hit(t.nickname)
                    || hit(t.name)
                    || hit(t.description)
                    || hit(t.media_class)
            }
            Field::Title => hit(t.title),
            Field::Nickname => hit(t.nickname),
            Field::Name => hit(t.name),
            Field::NameExact => {
                if let Some((n, mc)) = self.pattern.split_once('|') {
                    t.name.eq_ignore_ascii_case(n) && t.media_class.eq_ignore_ascii_case(mc)
                } else {
                    t.name.eq_ignore_ascii_case(&self.pattern)
                }
            }
            Field::Description => hit(t.description),
            Field::MediaClass => class_matches(&self.pattern, t.media_class),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub hidden: bool,
    pub color: Option<[u8; 3]>,
}

/// Apply the enabled rules to a node: Hide hides it, Show unhides it, and the
/// first matching Color colors it. Rules execute top to bottom.
pub fn evaluate<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
    t: &NodeText,
    default_hidden: bool,
) -> Outcome {
    let mut out = Outcome {
        hidden: default_hidden,
        color: None,
    };
    for r in rules.into_iter().filter(|r| r.enabled) {
        if !r.matches(t) {
            continue;
        }
        match r.action {
            Action::Hide => out.hidden = true,
            Action::Show => out.hidden = false,
            Action::Color(c) => {
                out.color.get_or_insert(c);
            }
        }
    }
    out
}

/// Name of the group that "Color" in a node's right-click menu writes to.
pub const MANUAL_COLORS: &str = "Manual colors";

/// A named bundle of rules with one on/off switch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleGroup {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// How a setup holds a group: its own copy, or a reference to a shared
/// library group (its rules live in the library file; `enabled` is per setup).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GroupEntry {
    Local(RuleGroup),
    Library { name: String, enabled: bool },
}

/// `name`, or `name (2)`, `name (3)`... if `name` is already taken.
pub fn unique_name(existing: &[String], name: &str) -> String {
    if !existing.iter().any(|e| e == name) {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|c| !existing.contains(c))
        .unwrap()
}

/// The setup's groups plus the shared library.
#[derive(Debug, Default)]
pub struct RuleSet {
    pub groups: Vec<GroupEntry>,
    pub library: Vec<RuleGroup>,
    /// The library changed and must be written to its file.
    pub library_dirty: bool,
}

impl RuleSet {
    fn group_rules<'a>(&'a self, e: &'a GroupEntry) -> &'a [Rule] {
        match e {
            GroupEntry::Local(g) if g.enabled => &g.rules,
            GroupEntry::Library { name, enabled: true } => self
                .library
                .iter()
                .find(|g| &g.name == name)
                .map_or(&[][..], |g| &g.rules[..]),
            _ => &[],
        }
    }

    /// All rules that are switched on (group and rule), in precedence order.
    pub fn active_rules(&self) -> impl Iterator<Item = &Rule> + '_ {
        self.groups
            .iter()
            .flat_map(move |e| self.group_rules(e).iter())
            .filter(|r| r.enabled)
    }

    /// The local group holding per-node colors, created first so that its
    /// colors win over every other Color rule.
    fn manual_colors(&mut self) -> &mut RuleGroup {
        let pos = self
            .groups
            .iter()
            .position(|e| matches!(e, GroupEntry::Local(g) if g.name == MANUAL_COLORS));
        let i = pos.unwrap_or_else(|| {
            let g = RuleGroup { name: MANUAL_COLORS.to_string(), enabled: true, rules: Vec::new() };
            self.groups.insert(0, GroupEntry::Local(g));
            0
        });
        match &mut self.groups[i] {
            GroupEntry::Local(g) => g,
            GroupEntry::Library { .. } => unreachable!("position() only finds Local groups"),
        }
    }

    /// Color one node (matched by its exact technical name).
    pub fn set_manual_color(&mut self, node_name: &str, color: [u8; 3]) {
        let g = self.manual_colors();
        g.enabled = true;
        let is_ours = |r: &Rule| {
            r.field == Field::NameExact
                && r.pattern == node_name
                && matches!(r.action, Action::Color(_))
        };
        match g.rules.iter_mut().find(|r| is_ours(r)) {
            Some(r) => {
                r.action = Action::Color(color);
                r.enabled = true;
            }
            None => g.rules.push(Rule {
                field: Field::NameExact,
                mode: Mode::Pattern,
                pattern: node_name.to_string(),
                action: Action::Color(color),
                enabled: true,
            }),
        }
    }

    pub fn clear_manual_color(&mut self, node_name: &str) {
        let bare = node_name.split_once('|').map(|(n, _)| n);
        for e in &mut self.groups {
            if let GroupEntry::Local(g) = e {
                if g.name == MANUAL_COLORS {
                    g.rules.retain(|r| {
                        !(r.field == Field::NameExact
                            && (r.pattern == node_name || bare.is_some_and(|b| r.pattern == b))
                            && matches!(r.action, Action::Color(_)))
                    });
                }
            }
        }
    }

    pub fn add_group(&mut self) {
        self.groups.push(GroupEntry::Local(RuleGroup {
            name: "New group".to_string(),
            enabled: true,
            rules: Vec::new(),
        }));
    }

    /// Move a local group into the shared library, keeping a reference here.
    pub fn to_library(&mut self, i: usize) {
        let Some(GroupEntry::Local(g)) = self.groups.get(i) else { return };
        let existing: Vec<String> = self.library.iter().map(|g| g.name.clone()).collect();
        let name = unique_name(&existing, &g.name);
        let enabled = g.enabled;
        let mut lib = g.clone();
        lib.name = name.clone();
        lib.enabled = true;
        self.library.push(lib);
        self.groups[i] = GroupEntry::Library { name, enabled };
        self.library_dirty = true;
    }

    /// Replace a library reference with a private copy of the group.
    pub fn unlink(&mut self, i: usize) {
        let Some(GroupEntry::Library { name, enabled }) = self.groups.get(i) else { return };
        let Some(src) = self.library.iter().find(|g| &g.name == name) else { return };
        let mut g = src.clone();
        g.enabled = *enabled;
        self.groups[i] = GroupEntry::Local(g);
    }

    /// Reference a library group from this setup (once).
    pub fn use_library(&mut self, name: &str) {
        let known = self.library.iter().any(|g| g.name == name);
        let used = self
            .groups
            .iter()
            .any(|e| matches!(e, GroupEntry::Library { name: n, .. } if n == name));
        if known && !used {
            self.groups.push(GroupEntry::Library { name: name.to_string(), enabled: true });
        }
    }

    /// Remove a group from the library. References here become private copies
    /// so no rule silently disappears.
    pub fn delete_from_library(&mut self, name: &str) {
        for i in 0..self.groups.len() {
            if matches!(&self.groups[i], GroupEntry::Library { name: n, .. } if n == name) {
                self.unlink(i);
            }
        }
        self.library.retain(|g| g.name != name);
        self.library_dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(field: Field, pattern: &str, action: Action) -> Rule {
        Rule { field, mode: Mode::Pattern, pattern: pattern.to_string(), action, enabled: true }
    }

    fn t<'a>(title: &'a str, name: &'a str, description: &'a str, class: &'a str) -> NodeText<'a> {
        NodeText { title, nickname: "", name, description, media_class: class }
    }

    #[test]
    fn glob_basics() {
        assert!(glob_match("*", ""));
        assert!(glob_match("alsa_*", "ALSA_output.pci"));
        assert!(glob_match("*monitor", "Built-in Monitor"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("a*b*c", "a--b--c"));
        assert!(!glob_match("a*b*c", "a--b--"));
        assert!(!glob_match("abc", "abcd"));
        assert!(glob_match("**x", "x"));
    }

    #[test]
    fn title_matches_what_the_node_shows() {
        // The shown title is the description; the technical name is different.
        let n = t("HP TrueVision HD Camera (V4L2)", "v4l2_input.pci-0000_00_14.0", "HP TrueVision HD Camera (V4L2)", "Video/Source");
        assert!(rule(Field::Title, "*HP*", Action::Hide).matches(&n));
        assert!(rule(Field::Title, "H*", Action::Hide).matches(&n));
        assert!(rule(Field::Title, "hp true", Action::Hide).matches(&n));
        assert!(!rule(Field::Name, "H*", Action::Hide).matches(&n));
        assert!(rule(Field::Name, "v4l2_*", Action::Hide).matches(&n));
        let alder = t("Alder Lake PCH-P Audio", "alsa_input.pci", "Alder Lake PCH-P Audio", "Audio/Source");
        assert!(rule(Field::Title, "Alder Lake *", Action::Hide).matches(&alder));
    }

    #[test]
    fn exact_name_is_not_contains() {
        let r = rule(Field::NameExact, "alsa_output.a", Action::Hide);
        assert!(r.matches(&t("", "ALSA_output.a", "", "")));
        assert!(!r.matches(&t("", "alsa_output.a.b", "", "")));
        // `*` is just a character here.
        assert!(!rule(Field::NameExact, "*", Action::Hide).matches(&t("", "x", "", "")));
    }

    #[test]
    fn manual_colors_win_and_can_be_cleared() {
        let mut set = RuleSet::default();
        set.groups.push(GroupEntry::Local(RuleGroup {
            name: "Mine".to_string(),
            enabled: true,
            rules: vec![rule(Field::Title, "mic", Action::Color([1, 1, 1]))],
        }));
        let node = t("Studio mic", "alsa_input.a", "", "");
        let color = |set: &RuleSet| evaluate(set.active_rules(), &node, false).color;
        assert_eq!(color(&set), Some([1, 1, 1]));

        set.set_manual_color("alsa_input.a", [9, 9, 9]);
        assert!(matches!(&set.groups[0], GroupEntry::Local(g) if g.name == MANUAL_COLORS));
        assert_eq!(color(&set), Some([9, 9, 9])); // manual beats the other rule

        set.set_manual_color("alsa_input.a", [7, 7, 7]); // update, not duplicate
        assert_eq!(color(&set), Some([7, 7, 7]));
        assert!(matches!(&set.groups[0], GroupEntry::Local(g) if g.rules.len() == 1));
        assert_eq!(set.groups.len(), 2);

        set.set_manual_color("other", [5, 5, 5]); // other nodes unaffected
        assert_eq!(color(&set), Some([7, 7, 7]));

        set.clear_manual_color("alsa_input.a");
        assert_eq!(color(&set), Some([1, 1, 1]));

        // Composite key ("name|media_class") distinguishes nodes sharing a name.
        let discord_out = t("WEBRTC VoiceEngine", "WEBRTC VoiceEngine", "", "Stream/Output/Audio");
        let discord_in = t("WEBRTC VoiceEngine", "WEBRTC VoiceEngine", "", "Stream/Input/Audio");
        set.set_manual_color("WEBRTC VoiceEngine|Stream/Output/Audio", [10, 20, 30]);
        set.set_manual_color("WEBRTC VoiceEngine|Stream/Input/Audio", [40, 50, 60]);
        assert_eq!(evaluate(set.active_rules(), &discord_out, false).color, Some([10, 20, 30]));
        assert_eq!(evaluate(set.active_rules(), &discord_in, false).color, Some([40, 50, 60]));
        set.clear_manual_color("WEBRTC VoiceEngine|Stream/Output/Audio");
        assert_eq!(evaluate(set.active_rules(), &discord_out, false).color, None);
        assert_eq!(evaluate(set.active_rules(), &discord_in, false).color, Some([40, 50, 60]));
    }

    fn rule_mode(field: Field, mode: Mode, pattern: &str) -> Rule {
        Rule { mode, ..rule(field, pattern, Action::Hide) }
    }

    #[test]
    fn exact_mode_matches_whole_text_only() {
        let n = t("Midi-Bridge", "", "", "");
        assert!(rule_mode(Field::Title, Mode::Exact, "midi-bridge").matches(&n));
        assert!(!rule_mode(Field::Title, Mode::Exact, "midi").matches(&n));
        assert!(!rule_mode(Field::Title, Mode::Exact, "Midi-Bridge 2").matches(&n));
        // Wildcard characters are literal here.
        assert!(!rule_mode(Field::Title, Mode::Exact, "Midi*").matches(&n));
        assert!(rule_mode(Field::Title, Mode::Exact, "a*b").matches(&t("A*B", "", "", "")));
        // Works on other fields and on "any field".
        let m = t("x", "alsa_output.a", "Speaker", "Audio/Sink");
        assert!(rule_mode(Field::Description, Mode::Exact, "speaker").matches(&m));
        assert!(rule_mode(Field::Any, Mode::Exact, "Audio/Sink").matches(&m));
        assert!(!rule_mode(Field::Name, Mode::Exact, "alsa_output").matches(&m));
        // An empty pattern never matches by accident (use "Is empty" for that).
        assert!(!rule_mode(Field::Title, Mode::Exact, "").matches(&t("", "", "", "")));
    }

    #[test]
    fn empty_mode_finds_blank_fields() {
        let blank = rule_mode(Field::Title, Mode::Empty, "");
        assert!(blank.matches(&t("", "n", "d", "c")));
        assert!(blank.matches(&t("   ", "n", "d", "c")));
        assert!(!blank.matches(&t("x", "", "", "")));
        // Nickname empty = node has no nickname.
        let no_nick = rule_mode(Field::Nickname, Mode::Empty, "");
        assert!(no_nick.matches(&t("x", "", "", "")));
        let mut with_nick = t("x", "", "", "");
        with_nick.nickname = "mic";
        assert!(!no_nick.matches(&with_nick));
        // "Any field" empty only when every field is blank.
        let any = rule_mode(Field::Any, Mode::Empty, "");
        assert!(any.matches(&t("", "", " ", "")));
        assert!(!any.matches(&t("", "", "", "Audio/Sink")));
        // Usable as a Hide rule through evaluate.
        let o = evaluate(&[blank.clone()], &t("", "n", "", ""), false);
        assert!(o.hidden);
        assert!(!evaluate(&[blank], &t("ok", "", "", ""), false).hidden);
    }

    #[test]
    fn nickname_field() {
        let mut n = t("Alder Lake PCH-P", "alsa_input.pci", "", "");
        n.nickname = "Studio mic";
        assert!(rule(Field::Nickname, "mic", Action::Hide).matches(&n));
        assert!(rule(Field::Nickname, "?*", Action::Hide).matches(&n));
        assert!(rule(Field::Any, "studio", Action::Hide).matches(&n));
        assert!(!rule(Field::Title, "studio", Action::Hide).matches(&n));
        // No nickname: only a match-everything pattern hits, `?*` does not.
        let plain = t("Alder Lake PCH-P", "alsa_input.pci", "", "");
        assert!(!rule(Field::Nickname, "mic", Action::Hide).matches(&plain));
        assert!(!rule(Field::Nickname, "?*", Action::Hide).matches(&plain));
    }

    #[test]
    fn plain_pattern_matches_anywhere_wildcard_matches_whole() {
        let n = t("Midi-Bridge", "", "", "");
        assert!(rule(Field::Title, "midi", Action::Hide).matches(&n));
        assert!(rule(Field::Title, "midi*", Action::Hide).matches(&n));
        let m = t("my_midi_thing", "", "", "");
        assert!(rule(Field::Title, "midi", Action::Hide).matches(&m));
        assert!(!rule(Field::Title, "midi*", Action::Hide).matches(&m));
    }

    #[test]
    fn media_class_is_exact_unless_wildcard() {
        let src = t("", "", "", "Audio/Source");
        let virt = t("", "", "", "Audio/Source/Virtual");
        let r = rule(Field::MediaClass, "audio/source", Action::Hide);
        assert!(r.matches(&src));
        assert!(!r.matches(&virt));
        assert!(rule(Field::MediaClass, "Audio/Source*", Action::Hide).matches(&virt));
    }

    #[test]
    fn any_field_and_empty_pattern() {
        let any = rule(Field::Any, "sink", Action::Hide);
        assert!(any.matches(&t("sink", "", "", "")));
        assert!(any.matches(&t("", "", "", "Audio/Sink")));
        assert!(any.matches(&t("", "x_sink", "", "")));
        assert!(!rule(Field::Any, "", Action::Hide).matches(&t("a", "b", "c", "d")));
    }

    #[test]
    fn evaluate_combines_rules() {
        let red = Action::Color([255, 0, 0]);
        let blue = Action::Color([0, 0, 255]);
        let mut off = rule(Field::Title, "fire", red.clone());
        off.enabled = false;
        let rules = vec![
            off,
            rule(Field::Title, "fire", blue),
            rule(Field::Title, "fire", red),
            rule(Field::Description, "private", Action::Hide),
        ];
        let o = evaluate(&rules, &t("Firefox", "", "", ""), false);
        assert_eq!(o, Outcome { hidden: false, color: Some([0, 0, 255]) }); // first enabled color
        let o = evaluate(&rules, &t("Firefox", "", "Private tab", ""), false);
        assert!(o.hidden && o.color.is_some());
        assert_eq!(evaluate(&rules, &t("Other", "", "", ""), false), Outcome::default());
    }

    #[test]
    fn show_action_unhides_default_hidden_node() {
        let rules = vec![rule(Field::MediaClass, "Audio/Sink", Action::Show)];
        let sink = t("USB Audio", "alsa_output.usb", "", "Audio/Sink");
        let other = t("Mic", "alsa_input.mic", "", "Audio/Source");
        let o_sink = evaluate(&rules, &sink, true);
        assert!(!o_sink.hidden);
        let o_other = evaluate(&rules, &other, true);
        assert!(o_other.hidden);
    }

    fn local(name: &str, rules: Vec<Rule>) -> GroupEntry {
        GroupEntry::Local(RuleGroup { name: name.to_string(), enabled: true, rules })
    }

    fn patterns(set: &RuleSet) -> Vec<String> {
        set.active_rules().map(|r| r.pattern.clone()).collect()
    }

    #[test]
    fn unique_names() {
        let ex = vec!["Noise".to_string(), "Noise (2)".to_string()];
        assert_eq!(unique_name(&ex, "Other"), "Other");
        assert_eq!(unique_name(&ex, "Noise"), "Noise (3)");
    }

    #[test]
    fn groups_order_and_switches() {
        let mut set = RuleSet::default();
        set.groups.push(local("a", vec![rule(Field::Title, "one", Action::Hide)]));
        let mut off = RuleGroup { name: "b".into(), enabled: false, rules: vec![rule(Field::Title, "two", Action::Hide)] };
        set.groups.push(GroupEntry::Local(off.clone()));
        off.enabled = true;
        off.rules[0].enabled = false;
        off.rules.push(rule(Field::Title, "three", Action::Hide));
        set.groups.push(GroupEntry::Local(off));
        assert_eq!(patterns(&set), vec!["one", "three"]);
    }

    #[test]
    fn library_round_trip() {
        let mut set = RuleSet::default();
        set.groups.push(local("System noise", vec![rule(Field::Title, "plasma", Action::Hide)]));
        set.to_library(0);
        assert!(set.library_dirty);
        assert!(matches!(&set.groups[0], GroupEntry::Library { name, enabled: true } if name == "System noise"));
        assert_eq!(patterns(&set), vec!["plasma"]); // still applies via the reference

        // Library edits show up through the reference.
        set.library[0].rules.push(rule(Field::Title, "kded", Action::Hide));
        assert_eq!(patterns(&set), vec!["plasma", "kded"]);

        // The per-setup switch turns the reference off without touching the library.
        set.groups[0] = GroupEntry::Library { name: "System noise".into(), enabled: false };
        assert!(patterns(&set).is_empty());
        assert_eq!(set.library[0].rules.len(), 2);

        // Another setup can use the same group; using twice does nothing.
        let mut other = RuleSet { library: set.library.clone(), ..Default::default() };
        other.use_library("System noise");
        other.use_library("System noise");
        other.use_library("missing");
        assert_eq!(other.groups.len(), 1);
        assert_eq!(patterns(&other), vec!["plasma", "kded"]);
    }

    #[test]
    fn unlink_and_delete_keep_rules() {
        let mut set = RuleSet::default();
        set.groups.push(local("G", vec![rule(Field::Title, "x", Action::Hide)]));
        set.to_library(0);
        set.groups.push(GroupEntry::Library { name: "G".into(), enabled: true });
        set.unlink(1);
        assert!(matches!(&set.groups[1], GroupEntry::Local(g) if g.name == "G" && g.rules.len() == 1));
        // Deleting from the library turns the remaining reference into a copy.
        set.delete_from_library("G");
        assert!(set.library.is_empty());
        assert!(matches!(&set.groups[0], GroupEntry::Local(g) if g.rules.len() == 1));
        assert_eq!(patterns(&set), vec!["x", "x"]);
        // A reference whose library group is missing contributes nothing.
        set.groups.push(GroupEntry::Library { name: "gone".into(), enabled: true });
        assert_eq!(patterns(&set).len(), 2);
        // Name clash when moving a second group of the same name to the library.
        set.groups.push(local("G", vec![]));
        set.to_library(set.groups.len() - 1);
        set.groups.push(local("G", vec![]));
        set.to_library(set.groups.len() - 1);
        let names: Vec<_> = set.library.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["G", "G (2)"]);
    }
}
