use eframe::egui::Color32;
use serde::Deserialize;
use std::path::Path;

fn parse_hex_color(hex: &str) -> Option<Color32> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color32::from_rgb(r, g, b))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color32::from_rgba_unmultiplied(r, g, b, a))
        }
        _ => None,
    }
}

fn color_or(raw: &Option<String>, default: Color32) -> Color32 {
    raw.as_deref().and_then(parse_hex_color).unwrap_or(default)
}

// ─── Raw YAML structs ──────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawMcp {
    port: Option<u16>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawManifest {
    app: RawApp,
    mcp: RawMcp,
    platforms: RawPlatforms,
    window: RawWindow,
    theme: Option<String>,
    font: RawFont,
    colors: RawColors,
    colors_dark: RawColors,
    sidebar: RawSidebar,
    dialog: RawDialog,
    input: RawInput,
    spacing: RawSpacing,
    corner_radius: RawCornerRadius,
    animations: RawAnimations,
    density: Option<String>,
    notifications: RawNotifications,
    scrollbar: RawScrollbar,
    shadows: RawShadows,
    hover: RawHover,
    focus_ring: RawFocusRing,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawApp {
    id: Option<String>,
    display_name: Option<String>,
    icon: Option<String>,
    version: Option<String>,
    build: Option<String>,
    copyright: Option<String>,
    author: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawPlatforms {
    macos: RawMacos,
    windows: RawWindows,
    linux: RawLinux,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawMacos {
    bundle_id: Option<String>,
    category: Option<String>,
    minimum_version: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawWindows {
    app_id: Option<String>,
    store_category: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawLinux {
    desktop_id: Option<String>,
    categories: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawWindow {
    width: Option<f32>,
    height: Option<f32>,
    min_width: Option<f32>,
    min_height: Option<f32>,
    resizable: Option<bool>,
    always_on_top: Option<bool>,
    start_maximized: Option<bool>,
    start_fullscreen: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawFont {
    family: Option<String>,
    size_base: Option<f32>,
    size_heading: Option<f32>,
    size_title: Option<f32>,
    size_caption: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawColors {
    bg_primary: Option<String>,
    bg_surface: Option<String>,
    bg_elevated: Option<String>,
    bg_overlay: Option<String>,
    text_primary: Option<String>,
    text_secondary: Option<String>,
    text_on_accent: Option<String>,
    accent: Option<String>,
    accent_hover: Option<String>,
    accent_subtle: Option<String>,
    border: Option<String>,
    error_text: Option<String>,
    error_bg: Option<String>,
    error_border: Option<String>,
    toast_success: Option<String>,
    toast_bg: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSidebar {
    width: Option<f32>,
    bg: Option<String>,
    text: Option<String>,
    text_active: Option<String>,
    item_active_bg: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawDialog {
    bg: Option<String>,
    corner_radius: Option<f32>,
    margin: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawInput {
    bg: Option<String>,
    border: Option<String>,
    corner_radius: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSpacing {
    content_margin_x: Option<f32>,
    content_margin_y: Option<f32>,
    titlebar_height: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawCornerRadius {
    button: Option<f32>,
    card: Option<f32>,
    badge: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawAnimations {
    toast_duration: Option<f32>,
    dialog_open: Option<f32>,
    dialog_close: Option<f32>,
    screen_transition: Option<f32>,
    node_appear_stagger: Option<f32>,
    node_appear_duration: Option<f32>,
    select_open: Option<f32>,
    select_close: Option<f32>,
    select_item_stagger: Option<f32>,
    select_flash: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawNotifications {
    position: Option<String>,
    max_visible: Option<u8>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawScrollbar {
    width: Option<f32>,
    auto_hide: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawShadows {
    enabled: Option<bool>,
    intensity: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawHover {
    scale: Option<f32>,
    transition: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawFocusRing {
    color: Option<String>,
    width: Option<f32>,
    offset: Option<f32>,
}

// ─── Resolved config ───────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Theme {
    Light,
    Dark,
    System,
}

#[derive(Clone, Copy, PartialEq)]
pub enum NotificationPosition {
    TopRight,
    BottomRight,
    BottomCenter,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Density {
    Compact,
    Comfortable,
    Spacious,
}

impl Density {
    pub fn factor(self) -> f32 {
        match self {
            Density::Compact => 0.8,
            Density::Comfortable => 1.0,
            Density::Spacious => 1.25,
        }
    }
}

#[derive(Clone)]
pub struct ColorPalette {
    pub bg_primary: Color32,
    pub bg_surface: Color32,
    pub bg_elevated: Color32,
    pub bg_overlay: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_on_accent: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_subtle: Color32,
    pub border: Color32,
    pub error_text: Color32,
    pub error_bg: Color32,
    pub error_border: Color32,
    pub toast_success: Color32,
    pub toast_bg: Color32,
}

#[derive(Clone)]
pub struct AppConfig {
    pub app_id: Option<String>,
    pub display_name: Option<String>,
    pub icon_path: Option<String>,
    pub version: Option<String>,
    pub build: Option<String>,
    pub copyright: Option<String>,
    pub author: Option<String>,
    // macOS
    pub macos_bundle_id: Option<String>,
    pub macos_category: Option<String>,
    pub macos_minimum_version: Option<String>,
    // Windows
    pub windows_app_id: Option<String>,
    pub windows_store_category: Option<String>,
    // Linux
    pub linux_desktop_id: Option<String>,
    pub linux_categories: Vec<String>,
    pub window_width: f32,
    pub window_height: f32,
    pub window_min_width: f32,
    pub window_min_height: f32,
    pub window_resizable: bool,
    pub window_always_on_top: bool,
    pub window_start_maximized: bool,
    pub window_start_fullscreen: bool,
    pub theme: Theme,
    pub font_family: Option<String>,
    pub font_size_base: f32,
    pub font_size_heading: f32,
    pub font_size_title: f32,
    pub font_size_caption: f32,
    pub colors: ColorPalette,
    pub colors_dark: ColorPalette,
    pub sidebar_width: f32,
    pub sidebar_bg: Color32,
    pub sidebar_text: Color32,
    pub sidebar_text_active: Color32,
    pub sidebar_item_active_bg: Color32,
    pub dialog_bg: Color32,
    pub dialog_corner_radius: f32,
    pub dialog_margin: f32,
    pub input_bg: Color32,
    pub input_border: Color32,
    pub input_corner_radius: f32,
    pub content_margin_x: f32,
    pub content_margin_y: f32,
    pub titlebar_height: f32,
    pub corner_radius_button: f32,
    pub corner_radius_card: f32,
    pub corner_radius_badge: f32,
    pub anim_toast_duration: f32,
    pub anim_dialog_open: f32,
    pub anim_dialog_close: f32,
    pub anim_screen_transition: f32,
    pub anim_node_appear_stagger: f32,
    pub anim_node_appear_duration: f32,
    pub anim_select_open: f32,
    pub anim_select_close: f32,
    pub anim_select_item_stagger: f32,
    pub anim_select_flash: f32,
    pub density: Density,
    pub notification_position: NotificationPosition,
    pub notification_max_visible: u8,
    pub scrollbar_width: f32,
    pub scrollbar_auto_hide: bool,
    pub shadows_enabled: bool,
    pub shadows_intensity: f32,
    pub hover_scale: f32,
    pub hover_transition: f32,
    pub focus_ring_color: Option<Color32>,
    pub focus_ring_width: f32,
    pub focus_ring_offset: f32,
    /// TCP port for the built-in MCP HTTP server. 0 = disabled (default).
    pub mcp_port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app_id: None,
            display_name: None,
            icon_path: None,
            version: None,
            build: None,
            copyright: None,
            author: None,
            macos_bundle_id: None,
            macos_category: None,
            macos_minimum_version: None,
            windows_app_id: None,
            windows_store_category: None,
            linux_desktop_id: None,
            linux_categories: Vec::new(),
            window_width: 1080.0,
            window_height: 720.0,
            window_min_width: 640.0,
            window_min_height: 480.0,
            window_resizable: true,
            window_always_on_top: false,
            window_start_maximized: false,
            window_start_fullscreen: false,
            theme: Theme::Light,
            font_family: None,
            font_size_base: 14.0,
            font_size_heading: 18.0,
            font_size_title: 24.0,
            font_size_caption: 12.0,
            colors: ColorPalette::default_light(),
            colors_dark: ColorPalette::default_dark(),
            sidebar_width: 220.0,
            sidebar_bg: Color32::from_rgb(0xF3, 0xF4, 0xF6),
            sidebar_text: Color32::from_rgb(0x4B, 0x50, 0x63),
            sidebar_text_active: Color32::from_rgb(0x1A, 0x1A, 0x2E),
            sidebar_item_active_bg: Color32::WHITE,
            dialog_bg: Color32::WHITE,
            dialog_corner_radius: 16.0,
            dialog_margin: 28.0,
            input_bg: Color32::WHITE,
            input_border: Color32::from_rgb(0xD1, 0xD5, 0xDB),
            input_corner_radius: 8.0,
            content_margin_x: 40.0,
            content_margin_y: 32.0,
            titlebar_height: 32.0,
            corner_radius_button: 10.0,
            corner_radius_card: 12.0,
            corner_radius_badge: 6.0,
            anim_toast_duration: 3.5,
            anim_dialog_open: 0.35,
            anim_dialog_close: 0.2,
            anim_screen_transition: 0.3,
            anim_node_appear_stagger: 0.05,
            anim_node_appear_duration: 0.35,
            anim_select_open: 0.35,
            anim_select_close: 0.2,
            anim_select_item_stagger: 0.06,
            anim_select_flash: 0.25,
            density: Density::Comfortable,
            notification_position: NotificationPosition::BottomCenter,
            notification_max_visible: 3,
            scrollbar_width: 6.0,
            scrollbar_auto_hide: true,
            shadows_enabled: true,
            shadows_intensity: 0.08,
            hover_scale: 1.0,
            hover_transition: 0.15,
            focus_ring_color: None,
            focus_ring_width: 2.0,
            focus_ring_offset: 2.0,
            mcp_port: 0,
        }
    }
}

impl ColorPalette {
    pub fn default_light() -> Self {
        Self {
            bg_primary: Color32::from_rgb(0xFA, 0xFA, 0xFC),
            bg_surface: Color32::WHITE,
            bg_elevated: Color32::WHITE,
            bg_overlay: Color32::from_rgba_premultiplied(0x10, 0x10, 0x18, 180),
            text_primary: Color32::from_rgb(0x1A, 0x1A, 0x2E),
            text_secondary: Color32::from_rgb(0x6B, 0x70, 0x80),
            text_on_accent: Color32::WHITE,
            accent: Color32::from_rgb(0x2D, 0x5B, 0xE3),
            accent_hover: Color32::from_rgb(0x1E, 0x4B, 0xD1),
            accent_subtle: Color32::from_rgb(0xEB, 0xF0, 0xFD),
            border: Color32::from_rgb(0xE8, 0xEA, 0xED),
            error_text: Color32::from_rgb(0xDC, 0x26, 0x26),
            error_bg: Color32::from_rgb(0xFE, 0xF2, 0xF2),
            error_border: Color32::from_rgb(0xFE, 0xCA, 0xCA),
            toast_success: Color32::from_rgb(0x05, 0x96, 0x69),
            toast_bg: Color32::from_rgb(0x1A, 0x1A, 0x2E),
        }
    }

    pub fn default_dark() -> Self {
        Self {
            bg_primary: Color32::from_rgb(0x0F, 0x0F, 0x14),
            bg_surface: Color32::from_rgb(0x1A, 0x1A, 0x24),
            bg_elevated: Color32::from_rgb(0x22, 0x22, 0x2E),
            bg_overlay: Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 200),
            text_primary: Color32::from_rgb(0xF0, 0xF0, 0xF5),
            text_secondary: Color32::from_rgb(0x9A, 0x9A, 0xAA),
            text_on_accent: Color32::WHITE,
            accent: Color32::from_rgb(0x5B, 0x8D, 0xF8),
            accent_hover: Color32::from_rgb(0x7A, 0xA5, 0xFA),
            accent_subtle: Color32::from_rgb(0x1E, 0x29, 0x3D),
            border: Color32::from_rgb(0x2E, 0x2E, 0x3A),
            error_text: Color32::from_rgb(0xF8, 0x71, 0x71),
            error_bg: Color32::from_rgb(0x2D, 0x15, 0x15),
            error_border: Color32::from_rgb(0x5C, 0x22, 0x22),
            toast_success: Color32::from_rgb(0x34, 0xD3, 0x99),
            toast_bg: Color32::from_rgb(0x1A, 0x1A, 0x2E),
        }
    }
}

// ─── Loading ───────────────────────────────────────────────────────────────

pub fn load_config(app_dir: &Path) -> AppConfig {
    let manifest_path = app_dir.join("manifest.yml");
    let raw: RawManifest = if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path).unwrap_or_default();
        serde_yaml::from_str(&content).unwrap_or_default()
    } else {
        RawManifest::default()
    };

    let theme = match raw.theme.as_deref() {
        Some("dark") => Theme::Dark,
        Some("system") => Theme::System,
        _ => Theme::Light,
    };

    let density = match raw.density.as_deref() {
        Some("compact") => Density::Compact,
        Some("spacious") => Density::Spacious,
        _ => Density::Comfortable,
    };

    let notification_position = match raw.notifications.position.as_deref() {
        Some("top-right") => NotificationPosition::TopRight,
        Some("bottom-right") => NotificationPosition::BottomRight,
        _ => NotificationPosition::BottomCenter,
    };

    let light_defaults = ColorPalette::default_light();
    let dark_defaults = ColorPalette::default_dark();

    let colors = resolve_palette(&raw.colors, &light_defaults);
    let colors_dark = resolve_palette(&raw.colors_dark, &dark_defaults);

    AppConfig {
        app_id: raw.app.id,
        display_name: raw.app.display_name,
        icon_path: raw.app.icon,
        version: raw.app.version,
        build: raw.app.build,
        copyright: raw.app.copyright,
        author: raw.app.author,
        macos_bundle_id: raw.platforms.macos.bundle_id,
        macos_category: raw.platforms.macos.category,
        macos_minimum_version: raw.platforms.macos.minimum_version,
        windows_app_id: raw.platforms.windows.app_id,
        windows_store_category: raw.platforms.windows.store_category,
        linux_desktop_id: raw.platforms.linux.desktop_id,
        linux_categories: raw.platforms.linux.categories.unwrap_or_default(),
        window_width: raw.window.width.unwrap_or(1080.0),
        window_height: raw.window.height.unwrap_or(720.0),
        window_min_width: raw.window.min_width.unwrap_or(640.0),
        window_min_height: raw.window.min_height.unwrap_or(480.0),
        window_resizable: raw.window.resizable.unwrap_or(true),
        window_always_on_top: raw.window.always_on_top.unwrap_or(false),
        window_start_maximized: raw.window.start_maximized.unwrap_or(false),
        window_start_fullscreen: raw.window.start_fullscreen.unwrap_or(false),
        theme,
        font_family: raw.font.family,
        font_size_base: raw.font.size_base.unwrap_or(14.0),
        font_size_heading: raw.font.size_heading.unwrap_or(18.0),
        font_size_title: raw.font.size_title.unwrap_or(24.0),
        font_size_caption: raw.font.size_caption.unwrap_or(12.0),
        colors,
        colors_dark,
        sidebar_width: raw.sidebar.width.unwrap_or(220.0),
        sidebar_bg: color_or(&raw.sidebar.bg, light_defaults.bg_primary),
        sidebar_text: color_or(&raw.sidebar.text, Color32::from_rgb(0x4B, 0x50, 0x63)),
        sidebar_text_active: color_or(&raw.sidebar.text_active, Color32::from_rgb(0x1A, 0x1A, 0x2E)),
        sidebar_item_active_bg: color_or(&raw.sidebar.item_active_bg, Color32::WHITE),
        dialog_bg: color_or(&raw.dialog.bg, Color32::WHITE),
        dialog_corner_radius: raw.dialog.corner_radius.unwrap_or(16.0),
        dialog_margin: raw.dialog.margin.unwrap_or(28.0),
        input_bg: color_or(&raw.input.bg, Color32::WHITE),
        input_border: color_or(&raw.input.border, Color32::from_rgb(0xD1, 0xD5, 0xDB)),
        input_corner_radius: raw.input.corner_radius.unwrap_or(8.0),
        content_margin_x: raw.spacing.content_margin_x.unwrap_or(40.0),
        content_margin_y: raw.spacing.content_margin_y.unwrap_or(32.0),
        titlebar_height: raw.spacing.titlebar_height.unwrap_or(32.0),
        corner_radius_button: raw.corner_radius.button.unwrap_or(10.0),
        corner_radius_card: raw.corner_radius.card.unwrap_or(12.0),
        corner_radius_badge: raw.corner_radius.badge.unwrap_or(6.0),
        anim_toast_duration: raw.animations.toast_duration.unwrap_or(3.5),
        anim_dialog_open: raw.animations.dialog_open.unwrap_or(0.35),
        anim_dialog_close: raw.animations.dialog_close.unwrap_or(0.2),
        anim_screen_transition: raw.animations.screen_transition.unwrap_or(0.3),
        anim_node_appear_stagger: raw.animations.node_appear_stagger.unwrap_or(0.05),
        anim_node_appear_duration: raw.animations.node_appear_duration.unwrap_or(0.35),
        anim_select_open: raw.animations.select_open.unwrap_or(0.35),
        anim_select_close: raw.animations.select_close.unwrap_or(0.2),
        anim_select_item_stagger: raw.animations.select_item_stagger.unwrap_or(0.06),
        anim_select_flash: raw.animations.select_flash.unwrap_or(0.25),
        density,
        notification_position,
        notification_max_visible: raw.notifications.max_visible.unwrap_or(3),
        scrollbar_width: raw.scrollbar.width.unwrap_or(6.0),
        scrollbar_auto_hide: raw.scrollbar.auto_hide.unwrap_or(true),
        shadows_enabled: raw.shadows.enabled.unwrap_or(true),
        shadows_intensity: raw.shadows.intensity.unwrap_or(0.08),
        hover_scale: raw.hover.scale.unwrap_or(1.0),
        hover_transition: raw.hover.transition.unwrap_or(0.15),
        focus_ring_color: raw.focus_ring.color.as_deref().and_then(parse_hex_color),
        focus_ring_width: raw.focus_ring.width.unwrap_or(2.0),
        focus_ring_offset: raw.focus_ring.offset.unwrap_or(2.0),
        mcp_port: raw.mcp.port.unwrap_or(0),
    }
}

fn resolve_palette(raw: &RawColors, defaults: &ColorPalette) -> ColorPalette {
    ColorPalette {
        bg_primary: color_or(&raw.bg_primary, defaults.bg_primary),
        bg_surface: color_or(&raw.bg_surface, defaults.bg_surface),
        bg_elevated: color_or(&raw.bg_elevated, defaults.bg_elevated),
        bg_overlay: color_or(&raw.bg_overlay, defaults.bg_overlay),
        text_primary: color_or(&raw.text_primary, defaults.text_primary),
        text_secondary: color_or(&raw.text_secondary, defaults.text_secondary),
        text_on_accent: color_or(&raw.text_on_accent, defaults.text_on_accent),
        accent: color_or(&raw.accent, defaults.accent),
        accent_hover: color_or(&raw.accent_hover, defaults.accent_hover),
        accent_subtle: color_or(&raw.accent_subtle, defaults.accent_subtle),
        border: color_or(&raw.border, defaults.border),
        error_text: color_or(&raw.error_text, defaults.error_text),
        error_bg: color_or(&raw.error_bg, defaults.error_bg),
        error_border: color_or(&raw.error_border, defaults.error_border),
        toast_success: color_or(&raw.toast_success, defaults.toast_success),
        toast_bg: color_or(&raw.toast_bg, defaults.toast_bg),
    }
}
