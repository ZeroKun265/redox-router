//! Visual theme: colors for egui UI elements, canvas background, grid dots,
//! nodes, ports, cables, and selection highlights.
//!
//! Includes built-in dark and light themes, live updates, and JSON serialization.

use egui::{Color32, CornerRadius, Shadow, Stroke};
use serde::{Deserialize, Serialize};

fn default_name() -> String {
    "Redox Dark".to_string()
}
fn yes_bool() -> bool {
    true
}
fn grid_pattern_def() -> GridPattern {
    GridPattern::Dots
}
fn cable_thickness_owned_def() -> f32 {
    2.2
}
fn cable_thickness_switch_def() -> f32 {
    2.3
}
fn cable_thickness_external_def() -> f32 {
    1.5
}
fn cable_grad_owned_start_def() -> [u8; 3] {
    [92, 196, 230]
}
fn cable_grad_owned_end_def() -> [u8; 3] {
    [110, 210, 140]
}
fn cable_grad_switch_start_def() -> [u8; 3] {
    [186, 120, 255]
}
fn cable_grad_switch_mid_in_def() -> [u8; 3] {
    [220, 115, 215]
}
fn cable_grad_switch_mid_out_def() -> [u8; 3] {
    [220, 115, 215]
}
fn cable_grad_switch_end_def() -> [u8; 3] {
    [255, 110, 180]
}
fn cable_grad_external_start_def() -> [u8; 3] {
    [120, 120, 135]
}
fn cable_grad_external_end_def() -> [u8; 3] {
    [165, 165, 180]
}
fn node_glow_source_def() -> [u8; 3] {
    [46, 125, 98]
}
fn node_glow_stream_def() -> [u8; 3] {
    [84, 88, 112]
}
fn node_glow_sink_def() -> [u8; 3] {
    [176, 104, 52]
}
fn c_panel_bg_def() -> [u8; 3] {
    [28, 29, 35]
}
fn c_window_bg_def() -> [u8; 3] {
    [34, 36, 43]
}
fn c_faint_bg_def() -> [u8; 3] {
    [40, 42, 50]
}
fn c_extreme_bg_def() -> [u8; 3] {
    [20, 21, 26]
}
fn c_widget_bg_def() -> [u8; 3] {
    [48, 51, 61]
}
fn c_widget_hover_def() -> [u8; 3] {
    [62, 67, 80]
}
fn c_widget_active_def() -> [u8; 3] {
    [40, 100, 125]
}
fn c_accent_def() -> [u8; 3] {
    [92, 196, 230]
}
fn c_canvas_bg_def() -> [u8; 3] {
    [22, 23, 28]
}
fn c_grid_dot_def() -> [u8; 3] {
    [44, 46, 56]
}
fn c_node_body_def() -> [u8; 3] {
    [38, 40, 48]
}
fn c_node_stroke_def() -> [u8; 3] {
    [72, 74, 84]
}
fn c_node_text_def() -> [u8; 3] {
    [210, 212, 220]
}
fn c_col_source_def() -> [u8; 3] {
    [46, 125, 98]
}
fn c_col_stream_def() -> [u8; 3] {
    [84, 88, 112]
}
fn c_col_sink_def() -> [u8; 3] {
    [176, 104, 52]
}
fn c_port_out_def() -> [u8; 3] {
    [110, 210, 140]
}
fn c_port_in_def() -> [u8; 3] {
    [240, 170, 90]
}
fn c_cable_owned_def() -> [u8; 3] {
    [92, 196, 230]
}
fn c_cable_switch_def() -> [u8; 3] {
    [186, 120, 255]
}
fn c_cable_external_def() -> [u8; 3] {
    [150, 150, 165]
}
fn c_cable_selected_def() -> [u8; 3] {
    [255, 206, 84]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridPattern {
    Dots,
    Lines,
    None,
}

impl Default for GridPattern {
    fn default() -> Self {
        Self::Dots
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwitchGradMode {
    Simple,
    Advanced,
}

impl Default for SwitchGradMode {
    fn default() -> Self {
        Self::Simple
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    /// Friendly name of this theme (e.g. "Redox Dark", "Redox Light").
    #[serde(default = "default_name")]
    pub name: String,

    /// Whether this theme is based on dark or light visuals.
    #[serde(default = "yes_bool")]
    pub is_dark: bool,

    /// Extra points added to all UI typography (labels, buttons, headers, inputs). Default: 0.0 (base is already +2pt).
    #[serde(default)]
    pub ui_font_size_delta: f32,

    /// Extra points added to graph node titles, port labels, and volume sliders. Default: 0.0.
    #[serde(default)]
    pub node_font_size_delta: f32,

    // --- UI Visuals ---
    #[serde(default = "c_panel_bg_def")]
    pub panel_bg: [u8; 3],
    #[serde(default = "c_window_bg_def")]
    pub window_bg: [u8; 3],
    #[serde(default = "c_faint_bg_def")]
    pub faint_bg: [u8; 3],
    #[serde(default = "c_extreme_bg_def")]
    pub extreme_bg: [u8; 3],
    #[serde(default = "c_widget_bg_def")]
    pub widget_bg: [u8; 3],
    #[serde(default = "c_widget_hover_def")]
    pub widget_hover: [u8; 3],
    #[serde(default = "c_widget_active_def")]
    pub widget_active: [u8; 3],
    #[serde(default = "c_accent_def")]
    pub accent: [u8; 3],

    // --- Canvas ---
    #[serde(default = "c_canvas_bg_def")]
    pub canvas_bg: [u8; 3],
    #[serde(default = "c_grid_dot_def")]
    pub grid_dot: [u8; 3],
    #[serde(default = "grid_pattern_def")]
    pub grid_pattern: GridPattern,

    // --- Nodes ---
    #[serde(default = "c_node_body_def")]
    pub node_body: [u8; 3],
    #[serde(default = "c_node_stroke_def")]
    pub node_stroke: [u8; 3],
    #[serde(default = "c_node_text_def")]
    pub node_text: [u8; 3],
    #[serde(default = "c_col_source_def")]
    pub col_source: [u8; 3],
    #[serde(default = "c_col_stream_def")]
    pub col_stream: [u8; 3],
    #[serde(default = "c_col_sink_def")]
    pub col_sink: [u8; 3],

    // --- Node Activity Glow ---
    #[serde(default = "yes_bool")]
    pub node_glow_enabled: bool,
    #[serde(default = "node_glow_source_def")]
    pub node_glow_source: [u8; 3],
    #[serde(default = "node_glow_stream_def")]
    pub node_glow_stream: [u8; 3],
    #[serde(default = "node_glow_sink_def")]
    pub node_glow_sink: [u8; 3],

    // --- Ports ---
    #[serde(default = "c_port_out_def")]
    pub port_out: [u8; 3],
    #[serde(default = "c_port_in_def")]
    pub port_in: [u8; 3],

    // --- Cables / Links ---
    #[serde(default = "c_cable_owned_def")]
    pub cable_owned: [u8; 3],
    #[serde(default = "c_cable_switch_def")]
    pub cable_switch: [u8; 3],
    #[serde(default = "c_cable_external_def")]
    pub cable_external: [u8; 3],
    #[serde(default = "c_cable_selected_def")]
    pub cable_selected: [u8; 3],

    // --- Link Effects: Thickness ---
    #[serde(default = "cable_thickness_owned_def")]
    pub cable_thickness_owned: f32,
    #[serde(default = "cable_thickness_switch_def")]
    pub cable_thickness_switch: f32,
    #[serde(default = "cable_thickness_external_def")]
    pub cable_thickness_external: f32,

    // --- Link Effects: Bloom / Glow Stroke ---
    #[serde(default = "yes_bool")]
    pub cable_glow_owned: bool,
    #[serde(default = "yes_bool")]
    pub cable_glow_switch: bool,
    #[serde(default = "yes_bool")]
    pub cable_glow_external: bool,

    // --- Link Effects: Gradient ---
    #[serde(default)]
    pub cable_grad_owned_enabled: bool,
    #[serde(default = "cable_grad_owned_start_def")]
    pub cable_grad_owned_start: [u8; 3],
    #[serde(default = "cable_grad_owned_end_def")]
    pub cable_grad_owned_end: [u8; 3],

    #[serde(default)]
    pub cable_grad_switch_enabled: bool,
    #[serde(default)]
    pub cable_grad_switch_mode: SwitchGradMode,
    #[serde(default = "cable_grad_switch_start_def")]
    pub cable_grad_switch_start: [u8; 3],
    #[serde(default = "cable_grad_switch_mid_in_def")]
    pub cable_grad_switch_mid_in: [u8; 3],
    #[serde(default = "cable_grad_switch_mid_out_def")]
    pub cable_grad_switch_mid_out: [u8; 3],
    #[serde(default = "cable_grad_switch_end_def")]
    pub cable_grad_switch_end: [u8; 3],

    #[serde(default)]
    pub cable_grad_external_enabled: bool,
    #[serde(default = "cable_grad_external_start_def")]
    pub cable_grad_external_start: [u8; 3],
    #[serde(default = "cable_grad_external_end_def")]
    pub cable_grad_external_end: [u8; 3],
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Built-in dark theme (the original RedoxRouter palette).
    pub fn dark() -> Self {
        Self {
            name: "Redox Dark".to_string(),
            is_dark: true,
            ui_font_size_delta: 0.0,
            node_font_size_delta: 0.0,
            panel_bg: [28, 29, 35],
            window_bg: [34, 36, 43],
            faint_bg: [40, 42, 50],
            extreme_bg: [20, 21, 26],
            widget_bg: [48, 51, 61],
            widget_hover: [62, 67, 80],
            widget_active: [40, 100, 125],
            accent: [92, 196, 230],
            canvas_bg: [22, 23, 28],
            grid_dot: [44, 46, 56],
            grid_pattern: GridPattern::Dots,
            node_body: [38, 40, 48],
            node_stroke: [72, 74, 84],
            node_text: [210, 212, 220],
            col_source: [46, 125, 98],
            col_stream: [84, 88, 112],
            col_sink: [176, 104, 52],
            node_glow_enabled: true,
            node_glow_source: [46, 125, 98],
            node_glow_stream: [84, 88, 112],
            node_glow_sink: [176, 104, 52],
            port_out: [110, 210, 140],
            port_in: [240, 170, 90],
            cable_owned: [92, 196, 230],
            cable_switch: [186, 120, 255],
            cable_external: [150, 150, 165],
            cable_selected: [255, 206, 84],
            cable_thickness_owned: 2.2,
            cable_thickness_switch: 2.3,
            cable_thickness_external: 1.5,
            cable_glow_owned: true,
            cable_glow_switch: true,
            cable_glow_external: true,
            cable_grad_owned_enabled: false,
            cable_grad_owned_start: [92, 196, 230],
            cable_grad_owned_end: [110, 210, 140],
            cable_grad_switch_enabled: false,
            cable_grad_switch_mode: SwitchGradMode::Simple,
            cable_grad_switch_start: [186, 120, 255],
            cable_grad_switch_mid_in: [220, 115, 215],
            cable_grad_switch_mid_out: [220, 115, 215],
            cable_grad_switch_end: [255, 110, 180],
            cable_grad_external_enabled: false,
            cable_grad_external_start: [120, 120, 135],
            cable_grad_external_end: [165, 165, 180],
        }
    }

    /// Built-in clean light theme.
    pub fn light() -> Self {
        Self {
            name: "Redox Light".to_string(),
            is_dark: false,
            ui_font_size_delta: 0.0,
            node_font_size_delta: 0.0,
            panel_bg: [232, 234, 239],
            window_bg: [246, 247, 250],
            faint_bg: [226, 229, 235],
            extreme_bg: [255, 255, 255],
            widget_bg: [218, 222, 228],
            widget_hover: [204, 210, 218],
            widget_active: [170, 205, 225],
            accent: [20, 135, 175],
            canvas_bg: [242, 244, 247],
            grid_dot: [200, 205, 214],
            grid_pattern: GridPattern::Dots,
            node_body: [255, 255, 255],
            node_stroke: [190, 195, 205],
            node_text: [40, 44, 52],
            col_source: [36, 130, 95],
            col_stream: [80, 95, 130],
            col_sink: [185, 100, 45],
            node_glow_enabled: true,
            node_glow_source: [36, 130, 95],
            node_glow_stream: [80, 95, 130],
            node_glow_sink: [185, 100, 45],
            port_out: [34, 150, 80],
            port_in: [215, 115, 25],
            cable_owned: [18, 130, 175],
            cable_switch: [145, 68, 215],
            cable_external: [135, 140, 150],
            cable_selected: [215, 145, 10],
            cable_thickness_owned: 2.2,
            cable_thickness_switch: 2.3,
            cable_thickness_external: 1.5,
            cable_glow_owned: true,
            cable_glow_switch: true,
            cable_glow_external: true,
            cable_grad_owned_enabled: false,
            cable_grad_owned_start: [18, 130, 175],
            cable_grad_owned_end: [34, 150, 80],
            cable_grad_switch_enabled: false,
            cable_grad_switch_mode: SwitchGradMode::Simple,
            cable_grad_switch_start: [145, 68, 215],
            cable_grad_switch_mid_in: [180, 64, 175],
            cable_grad_switch_mid_out: [180, 64, 175],
            cable_grad_switch_end: [215, 60, 140],
            cable_grad_external_enabled: false,
            cable_grad_external_start: [110, 115, 125],
            cable_grad_external_end: [160, 165, 175],
        }
    }

    /// Curated dark pastel theme based on Catppuccin Mocha.
    pub fn catppuccin() -> Self {
        Self {
            name: "Catppuccin Mocha".to_string(),
            is_dark: true,
            ui_font_size_delta: 0.0,
            node_font_size_delta: 0.0,
            panel_bg: [24, 24, 37],
            window_bg: [30, 30, 46],
            faint_bg: [49, 50, 68],
            extreme_bg: [17, 17, 27],
            widget_bg: [69, 71, 90],
            widget_hover: [88, 91, 112],
            widget_active: [137, 180, 250],
            accent: [203, 166, 247],
            canvas_bg: [30, 30, 46],
            grid_dot: [69, 71, 90],
            grid_pattern: GridPattern::Dots,
            node_body: [36, 37, 56],
            node_stroke: [88, 91, 112],
            node_text: [205, 214, 244],
            col_source: [166, 227, 161],
            col_stream: [137, 180, 250],
            col_sink: [250, 179, 135],
            node_glow_enabled: true,
            node_glow_source: [166, 227, 161],
            node_glow_stream: [137, 180, 250],
            node_glow_sink: [250, 179, 135],
            port_out: [166, 227, 161],
            port_in: [249, 226, 175],
            cable_owned: [137, 220, 235],
            cable_switch: [203, 166, 247],
            cable_external: [108, 112, 134],
            cable_selected: [249, 226, 175],
            cable_thickness_owned: 2.2,
            cable_thickness_switch: 2.3,
            cable_thickness_external: 1.5,
            cable_glow_owned: true,
            cable_glow_switch: true,
            cable_glow_external: true,
            cable_grad_owned_enabled: false,
            cable_grad_owned_start: [137, 220, 235],
            cable_grad_owned_end: [166, 227, 161],
            cable_grad_switch_enabled: false,
            cable_grad_switch_mode: SwitchGradMode::Simple,
            cable_grad_switch_start: [203, 166, 247],
            cable_grad_switch_mid_in: [224, 180, 239],
            cable_grad_switch_mid_out: [224, 180, 239],
            cable_grad_switch_end: [245, 194, 231],
            cable_grad_external_enabled: false,
            cable_grad_external_start: [88, 91, 112],
            cable_grad_external_end: [108, 112, 134],
        }
    }

    /// Curated arctic north-bluish theme based on Nord.
    pub fn nord() -> Self {
        Self {
            name: "Nord".to_string(),
            is_dark: true,
            ui_font_size_delta: 0.0,
            node_font_size_delta: 0.0,
            panel_bg: [36, 41, 51],
            window_bg: [46, 52, 64],
            faint_bg: [59, 66, 82],
            extreme_bg: [31, 35, 44],
            widget_bg: [67, 76, 94],
            widget_hover: [76, 86, 106],
            widget_active: [136, 192, 208],
            accent: [136, 192, 208],
            canvas_bg: [46, 52, 64],
            grid_dot: [76, 86, 106],
            grid_pattern: GridPattern::Dots,
            node_body: [59, 66, 82],
            node_stroke: [76, 86, 106],
            node_text: [236, 239, 244],
            col_source: [163, 190, 140],
            col_stream: [129, 161, 193],
            col_sink: [208, 135, 112],
            node_glow_enabled: true,
            node_glow_source: [163, 190, 140],
            node_glow_stream: [129, 161, 193],
            node_glow_sink: [208, 135, 112],
            port_out: [163, 190, 140],
            port_in: [235, 203, 139],
            cable_owned: [136, 192, 208],
            cable_switch: [180, 142, 173],
            cable_external: [115, 126, 148],
            cable_selected: [235, 203, 139],
            cable_thickness_owned: 2.2,
            cable_thickness_switch: 2.3,
            cable_thickness_external: 1.5,
            cable_glow_owned: true,
            cable_glow_switch: true,
            cable_glow_external: true,
            cable_grad_owned_enabled: false,
            cable_grad_owned_start: [136, 192, 208],
            cable_grad_owned_end: [143, 188, 187],
            cable_grad_switch_enabled: false,
            cable_grad_switch_mode: SwitchGradMode::Simple,
            cable_grad_switch_start: [180, 142, 173],
            cable_grad_switch_mid_in: [194, 138, 142],
            cable_grad_switch_mid_out: [194, 138, 142],
            cable_grad_switch_end: [208, 135, 112],
            cable_grad_external_enabled: false,
            cable_grad_external_start: [76, 86, 106],
            cable_grad_external_end: [115, 126, 148],
        }
    }

    /// Curated high-contrast neon theme based on Cyberpunk aesthetics.
    pub fn cyberpunk() -> Self {
        Self {
            name: "Cyberpunk".to_string(),
            is_dark: true,
            ui_font_size_delta: 0.0,
            node_font_size_delta: 0.0,
            panel_bg: [24, 14, 38],
            window_bg: [30, 18, 48],
            faint_bg: [42, 25, 68],
            extreme_bg: [12, 6, 20],
            widget_bg: [52, 30, 82],
            widget_hover: [74, 42, 115],
            widget_active: [0, 240, 255],
            accent: [255, 0, 128],
            canvas_bg: [15, 8, 26],
            grid_dot: [75, 35, 110],
            grid_pattern: GridPattern::Lines,
            node_body: [28, 16, 44],
            node_stroke: [255, 0, 128],
            node_text: [245, 245, 255],
            col_source: [0, 255, 160],
            col_stream: [0, 210, 255],
            col_sink: [255, 110, 0],
            node_glow_enabled: true,
            node_glow_source: [0, 255, 160],
            node_glow_stream: [0, 210, 255],
            node_glow_sink: [255, 110, 0],
            port_out: [0, 255, 160],
            port_in: [255, 220, 0],
            cable_owned: [0, 240, 255],
            cable_switch: [255, 0, 220],
            cable_external: [120, 90, 150],
            cable_selected: [255, 230, 0],
            cable_thickness_owned: 2.6,
            cable_thickness_switch: 2.8,
            cable_thickness_external: 1.8,
            cable_glow_owned: true,
            cable_glow_switch: true,
            cable_glow_external: true,
            cable_grad_owned_enabled: true,
            cable_grad_owned_start: [0, 240, 255],
            cable_grad_owned_end: [0, 255, 160],
            cable_grad_switch_enabled: true,
            cable_grad_switch_mode: SwitchGradMode::Simple,
            cable_grad_switch_start: [255, 0, 220],
            cable_grad_switch_mid_in: [255, 0, 174],
            cable_grad_switch_mid_out: [255, 0, 174],
            cable_grad_switch_end: [255, 0, 128],
            cable_grad_external_enabled: false,
            cable_grad_external_start: [100, 75, 130],
            cable_grad_external_end: [130, 100, 165],
        }
    }

    pub fn c32(rgb: [u8; 3]) -> Color32 {
        Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }

    pub fn c32_alpha(&self, rgb: [u8; 3], a: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], a)
    }

    pub fn c_panel_bg(&self) -> Color32 {
        Self::c32(self.panel_bg)
    }
    pub fn c_window_bg(&self) -> Color32 {
        Self::c32(self.window_bg)
    }
    pub fn c_faint_bg(&self) -> Color32 {
        Self::c32(self.faint_bg)
    }
    pub fn c_extreme_bg(&self) -> Color32 {
        Self::c32(self.extreme_bg)
    }
    pub fn c_widget_bg(&self) -> Color32 {
        Self::c32(self.widget_bg)
    }
    pub fn c_widget_hover(&self) -> Color32 {
        Self::c32(self.widget_hover)
    }
    pub fn c_widget_active(&self) -> Color32 {
        Self::c32(self.widget_active)
    }
    pub fn c_accent(&self) -> Color32 {
        Self::c32(self.accent)
    }
    pub fn c_canvas_bg(&self) -> Color32 {
        Self::c32(self.canvas_bg)
    }
    pub fn c_grid_dot(&self) -> Color32 {
        Self::c32(self.grid_dot)
    }
    pub fn c_node_body(&self) -> Color32 {
        Self::c32(self.node_body)
    }
    pub fn c_node_stroke(&self) -> Color32 {
        Self::c32(self.node_stroke)
    }
    pub fn c_node_text(&self) -> Color32 {
        Self::c32(self.node_text)
    }
    pub fn c_col_source(&self) -> Color32 {
        Self::c32(self.col_source)
    }
    pub fn c_col_stream(&self) -> Color32 {
        Self::c32(self.col_stream)
    }
    pub fn c_col_sink(&self) -> Color32 {
        Self::c32(self.col_sink)
    }
    pub fn c_port_out(&self) -> Color32 {
        Self::c32(self.port_out)
    }
    pub fn c_port_in(&self) -> Color32 {
        Self::c32(self.port_in)
    }
    pub fn c_cable_owned(&self) -> Color32 {
        Self::c32(self.cable_owned)
    }
    pub fn c_cable_switch(&self) -> Color32 {
        Self::c32(self.cable_switch)
    }
    pub fn c_cable_external(&self) -> Color32 {
        self.c32_alpha(self.cable_external, 170)
    }
    pub fn c_cable_selected(&self) -> Color32 {
        Self::c32(self.cable_selected)
    }
    pub fn c_node_glow_source(&self) -> Color32 {
        Self::c32(self.node_glow_source)
    }
    pub fn c_node_glow_stream(&self) -> Color32 {
        Self::c32(self.node_glow_stream)
    }
    pub fn c_node_glow_sink(&self) -> Color32 {
        Self::c32(self.node_glow_sink)
    }
    pub fn c_cable_grad_owned_start(&self) -> Color32 {
        Self::c32(self.cable_grad_owned_start)
    }
    pub fn c_cable_grad_owned_end(&self) -> Color32 {
        Self::c32(self.cable_grad_owned_end)
    }
    pub fn c_cable_grad_switch_start(&self) -> Color32 {
        Self::c32(self.cable_grad_switch_start)
    }
    pub fn c_cable_grad_switch_mid_in(&self) -> Color32 {
        Self::c32(self.cable_grad_switch_mid_in)
    }
    pub fn c_cable_grad_switch_mid_out(&self) -> Color32 {
        Self::c32(self.cable_grad_switch_mid_out)
    }
    pub fn c_cable_grad_switch_end(&self) -> Color32 {
        Self::c32(self.cable_grad_switch_end)
    }
    pub fn c_cable_grad_external_start(&self) -> Color32 {
        self.c32_alpha(self.cable_grad_external_start, 170)
    }
    pub fn c_cable_grad_external_end(&self) -> Color32 {
        self.c32_alpha(self.cable_grad_external_end, 170)
    }

    /// Apply this theme to the egui context.
    pub fn apply(&self, ctx: &egui::Context) {
        if self.is_dark {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }
        let accent = self.c_accent();
        let ui_delta = self.ui_font_size_delta;
        ctx.style_mut(|s| {
            use egui::{FontFamily, FontId, TextStyle};
            // All base UI text is 2 points larger than stock egui (e.g. Small 10->12, Body 12.5->14.5, Button 12.5->14.5, Heading 18->20, Monospace 12->14), plus the user's ui_font_size_delta.
            s.text_styles = [
                (TextStyle::Small, FontId::new((12.0 + ui_delta).max(8.0), FontFamily::Proportional)),
                (TextStyle::Body, FontId::new((14.5 + ui_delta).max(9.0), FontFamily::Proportional)),
                (TextStyle::Button, FontId::new((14.5 + ui_delta).max(9.0), FontFamily::Proportional)),
                (TextStyle::Heading, FontId::new((20.0 + ui_delta).max(12.0), FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new((14.0 + ui_delta).max(9.0), FontFamily::Monospace)),
            ].into();

            let v = &mut s.visuals;
            v.panel_fill = self.c_panel_bg();
            v.window_fill = self.c_window_bg();
            v.extreme_bg_color = self.c_extreme_bg();
            v.faint_bg_color = self.c_faint_bg();
            v.window_stroke = Stroke::new(
                1.0_f32,
                if self.is_dark {
                    Color32::from_gray(64)
                } else {
                    Color32::from_gray(190)
                },
            );
            v.window_corner_radius = CornerRadius::same(10);
            v.menu_corner_radius = CornerRadius::same(8);
            v.window_shadow = Shadow {
                offset: [0, 6],
                blur: 20,
                spread: 0,
                color: Color32::from_black_alpha(if self.is_dark { 110 } else { 40 }),
            };
            v.popup_shadow = Shadow {
                offset: [0, 4],
                blur: 14,
                spread: 0,
                color: Color32::from_black_alpha(if self.is_dark { 100 } else { 30 }),
            };
            v.selection.bg_fill = self.c_widget_active();
            v.selection.stroke = Stroke::new(1.0_f32, accent);
            v.hyperlink_color = accent;
            for w in [
                &mut v.widgets.noninteractive,
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                w.corner_radius = CornerRadius::same(12);
            }
            v.widgets.inactive.bg_fill = self.c_widget_bg();
            v.widgets.inactive.weak_bg_fill = self.c_widget_bg();
            v.widgets.hovered.bg_fill = self.c_widget_hover();
            v.widgets.hovered.weak_bg_fill = self.c_widget_hover();
            v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent);
            v.widgets.active.bg_fill = self.c_widget_active();
            v.widgets.active.weak_bg_fill = self.c_widget_active();
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(12.0, 5.0);
            s.spacing.window_margin = egui::Margin::same(12);
        });
    }

    /// Top/bottom panel frame styled with this theme.
    pub fn bar_frame(&self) -> egui::Frame {
        egui::Frame::NONE
            .fill(self.c_panel_bg())
            .stroke(Stroke::new(
                1.0_f32,
                if self.is_dark {
                    Color32::from_black_alpha(70)
                } else {
                    Color32::from_gray(215)
                },
            ))
            .inner_margin(egui::Margin::symmetric(14, 8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_is_dark() {
        let def = Theme::default();
        let dark = Theme::dark();
        assert_eq!(def, dark);
        assert!(def.is_dark);
    }

    #[test]
    fn light_theme_is_not_dark() {
        let light = Theme::light();
        assert!(!light.is_dark);
        assert_eq!(light.name, "Redox Light");
    }

    #[test]
    fn theme_json_round_trip() {
        let light = Theme::light();
        let json = serde_json::to_string(&light).unwrap();
        let parsed: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(light, parsed);
    }

    #[test]
    fn partial_json_falls_back_to_defaults() {
        let partial = r#"{"name": "Custom"}"#;
        let parsed: Theme = serde_json::from_str(partial).unwrap();
        assert_eq!(parsed.name, "Custom");
        assert_eq!(parsed.accent, [92, 196, 230]);
        assert!(parsed.is_dark);
        assert_eq!(parsed.ui_font_size_delta, 0.0);
        assert_eq!(parsed.node_font_size_delta, 0.0);
    }

    #[test]
    fn font_size_deltas_round_trip() {
        let mut t = Theme::dark();
        t.ui_font_size_delta = 2.5;
        t.node_font_size_delta = -1.0;
        let json = serde_json::to_string(&t).unwrap();
        let parsed: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.ui_font_size_delta, 2.5);
        assert_eq!(parsed.node_font_size_delta, -1.0);
    }

    #[test]
    fn curated_themes_exist_and_serialize() {
        for t in [Theme::dark(), Theme::light(), Theme::catppuccin(), Theme::nord(), Theme::cyberpunk()] {
            let json = serde_json::to_string(&t).unwrap();
            let parsed: Theme = serde_json::from_str(&json).unwrap();
            assert_eq!(t.name, parsed.name);
            assert_eq!(t.grid_pattern, parsed.grid_pattern);
            assert_eq!(t.cable_glow_owned, parsed.cable_glow_owned);
            assert_eq!(t.cable_grad_switch_mode, parsed.cable_grad_switch_mode);
            assert_eq!(t.cable_grad_switch_mid_in, parsed.cable_grad_switch_mid_in);
            assert_eq!(t.cable_grad_switch_mid_out, parsed.cable_grad_switch_mid_out);
        }
    }
}
