//! Native renderer for Scrawler manifests.
//!
//! This renderer handles only the four generic contract effects:
//!   - `state.set`          updates a key in the state dictionary,
//!   - `view.open`          marks a view as visible,
//!   - `view.close`         hides a view,
//!   - `notification.show`  displays a temporary notification.

use crate::config::{self, AppConfig, Theme};
use crate::ipc;
use crate::manifest::{ActionParameter, AppManifest, SemanticAction, SemanticNode, find_node_mut};
use crate::runtime::{InvokeContext, LuaRuntime};
use crate::storage::AppStorage;
use eframe::egui;
use serde_json::json;
use eframe::egui::{Color32, ColorImage, CornerRadius, FontId, Margin, Stroke, TextureHandle, Vec2};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;
use ureq;

// ─── Easing functions ───────────────────────────────────────────────────────

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_out_back(t: f32) -> f32 {
    let c1: f32 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

fn ease_out_quart(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(4)
}

fn ease_out_elastic(t: f32) -> f32 {
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let c4 = (2.0 * std::f32::consts::PI) / 3.0;
    2.0_f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c4).sin() + 1.0
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
        (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
    )
}

// ─── Platform utilities ─────────────────────────────────────────────────────

fn play_system_sound(name: &str) {
    #[cfg(target_os = "macos")]
    {
        let sound = match name {
            "error" => "Basso",
            "success" => "Glass",
            _ => "Funk",
        };
        let _ = std::process::Command::new("afplay")
            .arg(format!("/System/Library/Sounds/{sound}.aiff"))
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let script = match name {
            "error" => "[System.Media.SystemSounds]::Exclamation.Play()",
            "success" => "[System.Media.SystemSounds]::Asterisk.Play()",
            _ => "[System.Media.SystemSounds]::Beep.Play()",
        };
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", script])
            .creation_flags(0x08000000)
            .spawn();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = name;
    }
}

#[cfg(target_os = "macos")]
fn set_dock_badge(count: i64) {
    let label = if count <= 0 {
        String::new()
    } else {
        count.to_string()
    };
    let script = format!(
        "tell application \"System Events\" to set the badge of every process whose bundle identifier is (item 1 of (get value of attribute \"AXBundleIdentifier\" of (first application process whose frontmost is true))) to \"{}\"",
        label
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .spawn();
}

// ─── WebSocket thread ────────────────────────────────────────────────────────

fn ws_thread(
    conn_id: String,
    url: String,
    on_message: String,
    on_close: String,
    sender: Sender<WsMsg>,
) {
    use tungstenite::connect;
    use tungstenite::Message;

    let (cmd_tx, cmd_rx) = mpsc::channel::<String>();

    // Notify the main loop about the sender so `http.ws.send` can write.
    let _ = sender.send(WsMsg::Connected {
        conn_id: conn_id.clone(),
        tx: cmd_tx,
    });

    let (mut ws, _) = match connect(&url) {
        Ok(pair) => pair,
        Err(e) => {
            let _ = sender.send(WsMsg::Message {
                conn_id: conn_id.clone(),
                on_message: on_message.clone(),
                data: format!("{{\"error\":\"{}\"}}", e),
            });
            let _ = sender.send(WsMsg::Closed { conn_id, on_close });
            return;
        }
    };

    loop {
        // Flush any outbound messages first (non-blocking).
        while let Ok(text) = cmd_rx.try_recv() {
            if text == "__close__" {
                let _ = ws.close(None);
                let _ = sender.send(WsMsg::Closed { conn_id, on_close });
                return;
            }
            if ws.send(Message::Text(text.into())).is_err() {
                break;
            }
        }

        match ws.read() {
            Ok(Message::Text(text)) => {
                let _ = sender.send(WsMsg::Message {
                    conn_id: conn_id.clone(),
                    on_message: on_message.clone(),
                    data: text.to_string(),
                });
            }
            Ok(Message::Close(_)) | Err(_) => {
                let _ = sender.send(WsMsg::Closed {
                    conn_id,
                    on_close,
                });
                return;
            }
            Ok(_) => {}
        }
    }
}

// ─── Design tokens ──────────────────────────────────────────────────────────

#[allow(dead_code)]
const BG_PRIMARY: Color32 = Color32::from_rgb(0xFA, 0xFA, 0xFC);
#[allow(dead_code)]
const BG_SURFACE: Color32 = Color32::WHITE;
#[allow(dead_code)]
const BG_ELEVATED: Color32 = Color32::WHITE;
#[allow(dead_code)]
const BG_OVERLAY: Color32 = Color32::from_rgba_premultiplied(0x10, 0x10, 0x18, 180);

#[allow(dead_code)]
const TEXT_PRIMARY: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x2E);
#[allow(dead_code)]
const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x6B, 0x70, 0x80);
#[allow(dead_code)]
const TEXT_ON_ACCENT: Color32 = Color32::WHITE;

#[allow(dead_code)]
const ACCENT: Color32 = Color32::from_rgb(0x2D, 0x5B, 0xE3);
#[allow(dead_code)]
const ACCENT_HOVER: Color32 = Color32::from_rgb(0x1E, 0x4B, 0xD1);
#[allow(dead_code)]
const ACCENT_SUBTLE: Color32 = Color32::from_rgb(0xEB, 0xF0, 0xFD);

#[allow(dead_code)]
const BORDER: Color32 = Color32::from_rgb(0xE8, 0xEA, 0xED);

#[allow(dead_code)]
const SIDEBAR_BG: Color32 = Color32::from_rgb(0xF3, 0xF4, 0xF6);
#[allow(dead_code)]
const SIDEBAR_ACTIVE: Color32 = Color32::WHITE;
#[allow(dead_code)]
const SIDEBAR_TEXT: Color32 = Color32::from_rgb(0x4B, 0x50, 0x63);
#[allow(dead_code)]
const SIDEBAR_TEXT_ACTIVE: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x2E);

#[allow(dead_code)]
const TOAST_SUCCESS: Color32 = Color32::from_rgb(0x05, 0x96, 0x69);
#[allow(dead_code)]
const TOAST_BG: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x2E);

#[allow(dead_code)]
const ERROR_TEXT: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);
#[allow(dead_code)]
const ERROR_BG: Color32 = Color32::from_rgb(0xFE, 0xF2, 0xF2);
#[allow(dead_code)]
const ERROR_BORDER: Color32 = Color32::from_rgb(0xFE, 0xCA, 0xCA);

#[allow(dead_code)]
const DIALOG_BG: Color32 = Color32::WHITE;

#[allow(dead_code)]
const INPUT_BG: Color32 = Color32::WHITE;
#[allow(dead_code)]
const INPUT_BORDER: Color32 = Color32::from_rgb(0xD1, 0xD5, 0xDB);

#[allow(dead_code)]
const TITLEBAR_HEIGHT: f32 = 32.0;


const SPLASH_BG: Color32 = Color32::from_rgb(0x0A, 0x0A, 0x0F);

const TOAST_DURATION_SECS: f32 = 3.5;
const SPLASH_HOLD_AFTER_GIF: f32 = 1.5;
const SPLASH_FADE_OUT: f32 = 0.4;

// ─── Assets ─────────────────────────────────────────────────────────────────

const LUCIDE_FONT: &[u8] = include_bytes!("../assets/lucide.ttf");
const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");
const LOGO_INTRO_GIF: &[u8] = include_bytes!("../assets/logo-intro.gif");
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

fn icon_family() -> egui::FontFamily {
    egui::FontFamily::Name("icons".into())
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "lucide".to_owned(),
        egui::FontData::from_static(LUCIDE_FONT).into(),
    );
    fonts
        .families
        .insert(icon_family(), vec!["lucide".to_owned()]);
    ctx.set_fonts(fonts);
}

// ─── GIF decoder ────────────────────────────────────────────────────────────

struct GifFrame {
    image: ColorImage,
    delay_ms: u32,
}

fn decode_gif(data: &[u8]) -> Vec<GifFrame> {
    use image::codecs::gif::GifDecoder;
    use image::{AnimationDecoder, ImageDecoder, RgbaImage};
    use std::io::Cursor;

    let cursor = Cursor::new(data);
    let decoder = match GifDecoder::new(cursor) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let (screen_w, screen_h) = decoder.dimensions();
    let mut canvas = RgbaImage::new(screen_w, screen_h);
    let frames_iter = decoder.into_frames();
    let mut result = Vec::new();

    for frame_result in frames_iter {
        let frame = match frame_result {
            Ok(f) => f,
            Err(_) => continue,
        };

        let (numerator, _) = frame.delay().numer_denom_ms();
        let delay_ms = numerator;

        let buf = frame.buffer();
        let left = frame.left();
        let top = frame.top();

        for y in 0..buf.height() {
            for x in 0..buf.width() {
                let px = buf.get_pixel(x, y);
                let dx = x + left;
                let dy = y + top;
                if dx < screen_w && dy < screen_h {
                    canvas.put_pixel(dx, dy, *px);
                }
            }
        }

        let raw: &[u8] = canvas.as_raw();
        result.push(GifFrame {
            image: ColorImage::from_rgba_unmultiplied(
                [screen_w as usize, screen_h as usize],
                raw,
            ),
            delay_ms,
        });
    }

    result
}

fn load_png_image(data: &[u8]) -> ColorImage {
    let img = image::load_from_memory_with_format(data, image::ImageFormat::Png)
        .expect("logo.png should be valid")
        .into_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, &img)
}

// ─── App state ──────────────────────────────────────────────────────────────

fn load_icon_data(data: &[u8]) -> egui::IconData {
    let img = image::load_from_memory(data)
        .expect("app icon should be valid")
        .into_rgba8();
    egui::IconData {
        rgba: img.to_vec(),
        width: img.width(),
        height: img.height(),
    }
}

/// Launches the native window.
pub fn run_native_app(manifest: AppManifest, runtime: LuaRuntime, app_dir: &std::path::Path) -> eframe::Result {
    let cfg = config::load_config(app_dir);

    let title = cfg.display_name.clone().unwrap_or_else(|| manifest.name.clone());

    let app_icon = load_icon_data(APP_ICON_PNG);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([cfg.window_width, cfg.window_height])
            .with_min_inner_size([cfg.window_min_width, cfg.window_min_height])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_icon(app_icon),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |creation_context| {
            configure_fonts(&creation_context.egui_ctx);
            Ok(Box::new(SemanticApplication::new(
                manifest,
                runtime,
                cfg,
                &creation_context.egui_ctx,
            )))
        }),
    )
}

struct Toast {
    message: String,
    created_at: Instant,
}

// ─── Animation state ────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct ButtonAnim {
    press_start: Option<Instant>,
}

#[derive(Clone)]
struct DialogAnim {
    open_start: Instant,
    closing: bool,
    close_start: Option<Instant>,
}

const DIALOG_OPEN_DURATION: f32 = 0.35;
const DIALOG_CLOSE_DURATION: f32 = 0.2;

#[derive(Clone)]
struct SidebarItemAnim {
    hover_t: f32,
    active_t: f32,
}

impl Default for SidebarItemAnim {
    fn default() -> Self {
        Self {
            hover_t: 0.0,
            active_t: 0.0,
        }
    }
}

#[derive(Clone)]
struct ListItemAnim {
    hover_t: f32,
}

impl Default for ListItemAnim {
    fn default() -> Self {
        Self { hover_t: 0.0 }
    }
}

#[derive(Clone)]
struct ToggleAnim {
    position_t: f32,
}

impl Default for ToggleAnim {
    fn default() -> Self {
        Self { position_t: 0.0 }
    }
}

#[derive(Clone)]
struct CheckboxAnim {
    check_t: f32,
}

impl Default for CheckboxAnim {
    fn default() -> Self {
        Self { check_t: 0.0 }
    }
}

#[derive(Clone)]
struct NodeAppearAnim {
    first_seen: Instant,
}

#[derive(Clone)]
struct ScreenTransition {
    started: Instant,
}

const SCREEN_TRANSITION_DURATION: f32 = 0.3;

#[derive(Clone)]
struct SelectAnim {
    opened_at: Option<Instant>,
    closing_at: Option<Instant>,
    selected_flash: Option<(String, Instant)>,
}

impl Default for SelectAnim {
    fn default() -> Self {
        Self {
            opened_at: None,
            closing_at: None,
            selected_flash: None,
        }
    }
}

const SELECT_OPEN_DURATION: f32 = 0.35;
const SELECT_CLOSE_DURATION: f32 = 0.2;
const SELECT_ITEM_STAGGER: f32 = 0.06;
const SELECT_FLASH_DURATION: f32 = 0.25;

enum SplashPhase {
    PoweredByFadeIn { started: Instant },
    PlayingGif { current_frame: usize, frame_start: Instant },
    Hold { started: Instant },
    FadingOut { started: Instant },
    Done,
}

const POWERED_BY_ANIM: f32 = 1.2;

// Messages from a WebSocket thread to the main loop.
enum WsMsg {
    Connected { conn_id: String, tx: Sender<String> },
    Message { #[allow(dead_code)] conn_id: String, on_message: String, data: String },
    Closed { conn_id: String, on_close: String },
}

// Messages from async HTTP fetch threads to the main loop.
struct FetchMsg {
    callback: String,
    response: Value,
}

struct SemanticApplication {
    manifest: AppManifest,
    runtime: LuaRuntime,
    cfg: AppConfig,
    state: HashMap<String, String>,
    open_views: HashSet<String>,
    toast: Option<Toast>,
    error: Option<String>,
    form_values: HashMap<String, String>,
    remote_effects: Receiver<Value>,
    active_screen: usize,
    hidden_nodes: HashSet<String>,
    storage: AppStorage,
    window_title: String,
    // WebSocket infrastructure
    ws_effect_sender: Sender<WsMsg>,
    ws_effect_receiver: Receiver<WsMsg>,
    ws_senders: HashMap<String, Sender<String>>,
    // Async HTTP fetch infrastructure
    fetch_sender: Sender<FetchMsg>,
    fetch_receiver: Receiver<FetchMsg>,
    // Deferred viewport commands (window.minimize / window.close) queued
    // during apply_effect and flushed at the top of ui().
    deferred_viewport_cmds: Vec<egui::ViewportCommand>,
    // MCP HTTP server — active port (0 = disabled or failed)
    mcp_port: u16,
    // Whether the instructions modal is open
    mcp_modal_open: bool,
    // Splash screen
    splash_phase: SplashPhase,
    splash_frames: Vec<GifFrame>,
    splash_texture: TextureHandle,
    // Animation state
    button_anims: HashMap<String, ButtonAnim>,
    dialog_anims: HashMap<String, DialogAnim>,
    sidebar_anims: HashMap<usize, SidebarItemAnim>,
    list_item_anims: HashMap<String, ListItemAnim>,
    toggle_anims: HashMap<String, ToggleAnim>,
    checkbox_anims: HashMap<String, CheckboxAnim>,
    node_appear_anims: HashMap<String, NodeAppearAnim>,
    screen_transition: Option<ScreenTransition>,
    close_button_hover_t: f32,
    open_selects: HashSet<String>,
    select_anims: HashMap<String, SelectAnim>,
}

impl SemanticApplication {
    fn new(manifest: AppManifest, runtime: LuaRuntime, cfg: AppConfig, ctx: &egui::Context) -> Self {
        let (sender, remote_effects) = mpsc::channel();

        let _mcp_ok = ipc::start_effect_listener(sender).is_ok();

        let storage = AppStorage::new(&manifest.id);

        let mut state = HashMap::new();
        for entry in &manifest.state {
            state.insert(entry.id.clone(), entry.default.clone());
        }

        // Decode GIF frames
        let splash_frames = decode_gif(LOGO_INTRO_GIF);

        // Single reusable texture for animation (upload first frame)
        let initial_image = if splash_frames.is_empty() {
            load_png_image(LOGO_PNG)
        } else {
            splash_frames[0].image.clone()
        };
        let splash_texture = ctx.load_texture(
            "splash_frame",
            initial_image,
            egui::TextureOptions {
                magnification: egui::TextureFilter::Linear,
                minification: egui::TextureFilter::Linear,
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..Default::default()
            },
        );


        let splash_phase = if splash_frames.is_empty() {
            SplashPhase::Done
        } else {
            SplashPhase::PoweredByFadeIn {
                started: Instant::now(),
            }
        };

        let window_title = cfg.display_name.clone().unwrap_or_else(|| manifest.name.clone());
        let (ws_effect_sender, ws_effect_receiver) = mpsc::channel();
        let (fetch_sender, fetch_receiver) = mpsc::channel();

        // Start the built-in MCP HTTP server if a port is configured.
        let mcp_port = if cfg.mcp_port != 0 {
            match (runtime.rebuild(), manifest.clone()) {
                (Ok(rt), m) => {
                    match crate::mcp::start_http_server(m, rt, cfg.mcp_port) {
                        Ok(()) => cfg.mcp_port,
                        Err(_) => 0,
                    }
                }
                _ => 0,
            }
        } else {
            0
        };

        Self {
            manifest,
            runtime,
            cfg,
            state,
            open_views: HashSet::new(),
            toast: None,
            error: None,
            form_values: HashMap::new(),
            remote_effects,
            active_screen: 0,
            hidden_nodes: HashSet::new(),
            window_title,
            storage,
            ws_effect_sender,
            ws_effect_receiver,
            ws_senders: HashMap::new(),
            fetch_sender,
            fetch_receiver,
            deferred_viewport_cmds: Vec::new(),
            mcp_port,
            mcp_modal_open: false,
            splash_phase,
            splash_frames,
            splash_texture,
            button_anims: HashMap::new(),
            dialog_anims: HashMap::new(),
            sidebar_anims: HashMap::new(),
            list_item_anims: HashMap::new(),
            toggle_anims: HashMap::new(),
            checkbox_anims: HashMap::new(),
            node_appear_anims: HashMap::new(),
            screen_transition: None,
            close_button_hover_t: 0.0,
            open_selects: HashSet::new(),
            select_anims: HashMap::new(),
        }
    }

    fn palette(&self) -> &config::ColorPalette {
        match self.cfg.theme {
            Theme::Dark => &self.cfg.colors_dark,
            _ => &self.cfg.colors,
        }
    }

    // Shorthand accessors — avoids repeating self.palette().xxx at every call site.
    #[allow(dead_code)]
    fn c_bg_primary(&self)    -> Color32 { self.palette().bg_primary }
    fn c_bg_surface(&self)    -> Color32 { self.palette().bg_surface }
    fn c_text_primary(&self)  -> Color32 { self.palette().text_primary }
    fn c_text_secondary(&self)-> Color32 { self.palette().text_secondary }
    fn c_accent(&self)        -> Color32 { self.palette().accent }
    fn c_accent_hover(&self)  -> Color32 { self.palette().accent_hover }
    fn c_accent_subtle(&self) -> Color32 { self.palette().accent_subtle }
    fn c_border(&self)        -> Color32 { self.palette().border }
    fn c_error_text(&self)    -> Color32 { self.palette().error_text }
    fn c_error_bg(&self)      -> Color32 { self.palette().error_bg }
    fn c_error_border(&self)  -> Color32 { self.palette().error_border }
    fn c_text_on_accent(&self)-> Color32 { self.palette().text_on_accent }

    fn apply_effect(&mut self, effect: Value) {
        match (
            effect.get("effect").and_then(Value::as_str),
            effect.get("target").and_then(Value::as_str),
        ) {
            (Some("state.set"), Some(key)) => {
                let value = effect
                    .pointer("/payload/value")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                self.state.insert(key.into(), value.into());
            }
            (Some("view.open"), Some(view_id)) => {
                if let Some(values) = effect.pointer("/payload/state").and_then(Value::as_object) {
                    for (key, value) in values {
                        if let Some(value) = value.as_str() {
                            self.state.insert(key.clone(), value.into());
                        }
                    }
                }
                self.open_views.insert(view_id.into());
                self.dialog_anims.insert(
                    view_id.into(),
                    DialogAnim {
                        open_start: Instant::now(),
                        closing: false,
                        close_start: None,
                    },
                );
            }
            (Some("view.close"), Some(view_id)) => {
                if let Some(anim) = self.dialog_anims.get_mut(view_id) {
                    anim.closing = true;
                    anim.close_start = Some(Instant::now());
                } else {
                    self.open_views.remove(view_id);
                }
            }
            (Some("screen.navigate"), Some(screen_id)) => {
                let idx = self
                    .manifest
                    .nodes
                    .iter()
                    .filter(|n| n.role == "screen")
                    .position(|n| n.id == screen_id);
                if let Some(idx) = idx {
                    if idx != self.active_screen {
                        self.screen_transition = Some(ScreenTransition {
                            started: Instant::now(),
                        });
                        self.active_screen = idx;
                    }
                }
            }
            (Some("manifest.set_label"), Some(node_id)) => {
                if let Some(label) = effect.pointer("/payload/label").and_then(Value::as_str) {
                    if let Some(node) = find_node_mut(&mut self.manifest.nodes, node_id) {
                        node.label = label.into();
                    }
                }
            }
            (Some("manifest.set_visible"), Some(node_id)) => {
                let visible = effect
                    .pointer("/payload/visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                if visible {
                    self.hidden_nodes.remove(node_id);
                } else {
                    self.hidden_nodes.insert(node_id.into());
                }
            }
            (Some("manifest.set_options"), Some(node_id)) => {
                if let Some(options) = effect
                    .pointer("/payload/options")
                    .and_then(Value::as_array)
                {
                    let labels: Vec<String> = options
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect();
                    if let Some(node) = find_node_mut(&mut self.manifest.nodes, node_id) {
                        node.children.retain(|c| c.role != "option");
                        for label in labels {
                            node.children.push(crate::manifest::SemanticNode {
                                id: format!("{node_id}.option.{label}"),
                                role: "option".into(),
                                label,
                                bind: None,
                                icon: None,
                                placeholder: None,
                                disabled: false,
                                readonly: false,
                                variant: None,
                                aria_label: None,
                                layout: None,
                                gap: None,
                                padding: None,
                                width: None,
                                min_width: None,
                                min_height: None,
                                max_height: None,
                                columns: None,
                                wrap: false,
                                scroll: false,
                                grow: false,
                                actions: Vec::new(),
                                children: Vec::new(),
                            });
                        }
                    }
                }
            }
            (Some("notification.show"), _) => {
                if let Some(message) = effect
                    .pointer("/payload/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    self.toast = Some(Toast {
                        message,
                        created_at: Instant::now(),
                    });
                }
            }
            (Some("notification.os"), _) => {
                let title = effect.pointer("/payload/title").and_then(Value::as_str).unwrap_or("").to_owned();
                let body = effect.pointer("/payload/body").and_then(Value::as_str).unwrap_or("").to_owned();
                let _ = notify_rust::Notification::new().summary(&title).body(&body).show();
            }
            (Some("browser.open"), Some(url)) => {
                let _ = webbrowser::open(url);
            }
            (Some("clipboard.write"), _) => {
                if let Some(text) = effect.pointer("/payload/text").and_then(Value::as_str) {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(text);
                    }
                }
            }
            (Some("sound.play"), Some(name)) => {
                play_system_sound(name);
            }
            (Some("window.set_title"), _) => {
                if let Some(title) = effect.pointer("/payload/title").and_then(Value::as_str) {
                    self.window_title = title.to_owned();
                }
            }
            (Some("window.set_badge"), _) => {
                // macOS Dock badge — best-effort, ignored on other platforms.
                #[cfg(target_os = "macos")]
                if let Some(count) = effect.pointer("/payload/count").and_then(Value::as_i64) {
                    set_dock_badge(count);
                }
            }
            (Some("file.save"), Some(filename)) => {
                let filename = filename.to_owned();
                let content = effect.pointer("/payload/content").and_then(Value::as_str).unwrap_or("").to_owned();
                std::thread::spawn(move || {
                    if let Some(path) = rfd::FileDialog::new().set_file_name(&filename).save_file() {
                        let _ = std::fs::write(path, content);
                    }
                });
            }
            // Storage write effects
            (Some("storage.set"), Some(key)) => {
                if let Some(value) = effect.pointer("/payload/value").cloned() {
                    self.storage.kv_set(key, value);
                }
            }
            (Some("storage.delete"), Some(key)) => {
                self.storage.kv_delete(key);
            }
            (Some("storage.file.write"), Some(path)) => {
                if let Some(content) = effect.pointer("/payload/content").and_then(Value::as_str) {
                    self.storage.file_write(path, content);
                }
            }
            (Some("storage.file.delete"), Some(path)) => {
                self.storage.file_delete(path);
            }
            (Some("storage.dir.create"), Some(path)) => {
                self.storage.dir_create(path);
            }
            (Some("manifest.set_icon"), Some(node_id)) => {
                if let Some(icon) = effect.pointer("/payload/icon").and_then(Value::as_str) {
                    if let Some(node) = find_node_mut(&mut self.manifest.nodes, node_id) {
                        node.icon = Some(icon.to_owned());
                    }
                }
            }
            (Some("form.reset"), Some(node_id)) => {
                let prefix = format!("{node_id}.");
                self.form_values.retain(|k, _| !k.starts_with(&prefix));
            }
            (Some("window.minimize"), _) => {
                self.deferred_viewport_cmds.push(egui::ViewportCommand::Minimized(true));
            }
            (Some("window.close"), _) => {
                self.deferred_viewport_cmds.push(egui::ViewportCommand::Close);
            }
            (Some("http.fetch_async"), Some(callback)) => {
                let callback = callback.to_owned();
                let url = effect.pointer("/payload/url").and_then(Value::as_str).unwrap_or("").to_owned();
                let method = effect.pointer("/payload/method").and_then(Value::as_str).unwrap_or("GET").to_uppercase();
                let body = effect.pointer("/payload/body").and_then(Value::as_str).map(str::to_owned);
                let timeout_secs = effect.pointer("/payload/timeout").and_then(Value::as_u64).unwrap_or(30);
                let headers: Vec<(String, String)> = effect
                    .pointer("/payload/headers")
                    .and_then(Value::as_object)
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                            .collect()
                    })
                    .unwrap_or_default();
                let tx = self.fetch_sender.clone();
                std::thread::spawn(move || {
                    let mut req = ureq::request(&method, &url);
                    req = req.timeout(std::time::Duration::from_secs(timeout_secs));
                    for (k, v) in &headers {
                        req = req.set(k, v);
                    }
                    let response = match body {
                        Some(b) => req.send_string(&b),
                        None => req.call(),
                    };
                    let msg = match response {
                        Ok(resp) => {
                            let status = resp.status();
                            let mut resp_headers: Map<String, Value> = Map::new();
                            for name in resp.headers_names() {
                                if let Some(val) = resp.header(&name) {
                                    resp_headers.insert(name, Value::String(val.to_owned()));
                                }
                            }
                            let resp_body = resp.into_string().unwrap_or_default();
                            FetchMsg {
                                callback,
                                response: json!({
                                    "status": status,
                                    "body": resp_body,
                                    "headers": resp_headers,
                                    "ok": status >= 200 && status < 300
                                }),
                            }
                        }
                        Err(ureq::Error::Status(code, resp)) => {
                            let resp_body = resp.into_string().unwrap_or_default();
                            FetchMsg {
                                callback,
                                response: json!({
                                    "status": code,
                                    "body": resp_body,
                                    "headers": {},
                                    "ok": false
                                }),
                            }
                        }
                        Err(e) => FetchMsg {
                            callback,
                            response: json!({
                                "status": 0,
                                "body": e.to_string(),
                                "headers": {},
                                "ok": false,
                                "error": e.to_string()
                            }),
                        },
                    };
                    let _ = tx.send(msg);
                });
            }
            (Some("http.ws.connect"), Some(conn_id)) => {
                let url = effect.pointer("/payload/url").and_then(Value::as_str).unwrap_or("").to_owned();
                let on_message = effect.pointer("/payload/on_message").and_then(Value::as_str).unwrap_or("").to_owned();
                let on_close = effect.pointer("/payload/on_close").and_then(Value::as_str).unwrap_or("").to_owned();
                let conn_id = conn_id.to_owned();
                let sender = self.ws_effect_sender.clone();
                std::thread::spawn(move || {
                    ws_thread(conn_id, url, on_message, on_close, sender);
                });
            }
            (Some("http.ws.send"), Some(conn_id)) => {
                if let Some(data) = effect.pointer("/payload/data").and_then(Value::as_str) {
                    if let Some(tx) = self.ws_senders.get(conn_id) {
                        let _ = tx.send(data.to_owned());
                    }
                }
            }
            (Some("http.ws.close"), Some(conn_id)) => {
                self.ws_senders.remove(conn_id);
            }
            _ => {}
        }
        self.error = None;
    }

    fn invoke_action(&mut self, node: &SemanticNode, action: &SemanticAction) {
        let arguments = match self.collect_arguments(node, action) {
            Ok(args) => args,
            Err(err) => {
                self.error = Some(err);
                return;
            }
        };
        let ctx = InvokeContext::new(&self.state, &self.storage);
        match self.runtime.invoke(&action.handler, &arguments, &ctx) {
            Ok(effects) => {
                for effect in effects {
                    self.apply_effect(effect);
                }
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }

    fn collect_arguments(
        &self,
        node: &SemanticNode,
        action: &SemanticAction,
    ) -> Result<Map<String, Value>, String> {
        let mut arguments = Map::new();
        for parameter in &action.parameters {
            let key = form_key(&node.id, &action.id, &parameter.name);
            let text = self.form_values.get(&key).map(String::as_str).unwrap_or("");
            if text.is_empty() && !parameter.required {
                continue;
            }
            if text.is_empty() {
                return Err(format!("\"{}\" is required", parameter.name));
            }
            arguments.insert(parameter.name.clone(), coerce_value(text, parameter)?);
        }
        Ok(arguments)
    }

    fn screens(&self) -> Vec<&SemanticNode> {
        self.manifest
            .nodes
            .iter()
            .filter(|n| n.role == "screen")
            .collect()
    }

    // ─── Splash screen ───────────────────────────────────────────────────────

    fn render_splash(&mut self, ctx: &egui::Context) -> bool {
        let opacity = match &self.splash_phase {
            SplashPhase::Done => return false,
            SplashPhase::FadingOut { started } => {
                let elapsed = started.elapsed().as_secs_f32();
                if elapsed >= SPLASH_FADE_OUT {
                    self.splash_phase = SplashPhase::Done;
                    return false;
                }
                1.0 - (elapsed / SPLASH_FADE_OUT)
            }
            _ => 1.0,
        };

        // Advance phase
        match &self.splash_phase {
            SplashPhase::PoweredByFadeIn { started } => {
                if started.elapsed().as_secs_f32() >= POWERED_BY_ANIM {
                    self.splash_phase = SplashPhase::PlayingGif {
                        current_frame: 0,
                        frame_start: Instant::now(),
                    };
                }
            }
            SplashPhase::PlayingGif {
                current_frame,
                frame_start,
            } => {
                let frame_idx = *current_frame;
                let delay = self.splash_frames[frame_idx].delay_ms as f32 / 1000.0;
                if frame_start.elapsed().as_secs_f32() >= delay {
                    let next = frame_idx + 1;
                    if next >= self.splash_frames.len() {
                        self.splash_phase = SplashPhase::Hold {
                            started: Instant::now(),
                        };
                    } else {
                        self.splash_texture.set(
                            self.splash_frames[next].image.clone(),
                            egui::TextureOptions {
                                magnification: egui::TextureFilter::Linear,
                                minification: egui::TextureFilter::Linear,
                                mipmap_mode: Some(egui::TextureFilter::Linear),
                                ..Default::default()
                            },
                        );
                        self.splash_phase = SplashPhase::PlayingGif {
                            current_frame: next,
                            frame_start: Instant::now(),
                        };
                    }
                }
            }
            SplashPhase::Hold { started } => {
                if started.elapsed().as_secs_f32() >= SPLASH_HOLD_AFTER_GIF {
                    self.splash_phase = SplashPhase::FadingOut {
                        started: Instant::now(),
                    };
                }
            }
            _ => {}
        }

        let alpha = (opacity * 255.0) as u8;

        let screen_rect = ctx.viewport_rect();

        egui::Area::new(egui::Id::new("splash_screen"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Debug)
            .interactable(false)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen_rect.size(), egui::Sense::hover());

                let bg = Color32::from_rgba_unmultiplied(
                    SPLASH_BG.r(),
                    SPLASH_BG.g(),
                    SPLASH_BG.b(),
                    alpha,
                );
                ui.painter().rect_filled(rect, 0.0, bg);

                // GIF position (always computed for text placement)
                let texture = &self.splash_texture;
                let tex_size = texture.size_vec2();
                // tex_size is in physical pixels; convert to logical points before scaling
                let ppp = ui.ctx().pixels_per_point();
                let logical_tex_size = tex_size / ppp;
                let max_w = rect.width() * 0.4;
                let scale = (max_w / logical_tex_size.x).min(1.0);
                let display_size = logical_tex_size * scale;
                let img_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.center().x, rect.center().y + 10.0),
                    display_size,
                );

                // "powered by" — opacity animation, positioned above GIF
                let text_alpha = match &self.splash_phase {
                    SplashPhase::PoweredByFadeIn { started } => {
                        // Wait 0.3s then fade in over 0.9s
                        let elapsed = started.elapsed().as_secs_f32();
                        let t = ((elapsed - 0.3) / 0.9).clamp(0.0, 1.0);
                        (t * 255.0 * opacity) as u8
                    }
                    _ => alpha,
                };
                ui.painter().text(
                    egui::pos2(rect.center().x, img_rect.min.y - 2.0),
                    egui::Align2::CENTER_BOTTOM,
                    "powered by",
                    FontId::proportional(14.0),
                    Color32::from_rgba_unmultiplied(160, 160, 170, text_alpha),
                );

                // GIF (only after PoweredByFadeIn)
                let show_gif = !matches!(&self.splash_phase, SplashPhase::PoweredByFadeIn { .. });
                if show_gif {
                    let tint = Color32::from_rgba_unmultiplied(255, 255, 255, alpha);
                    ui.painter().image(
                        texture.id(),
                        img_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        tint,
                    );
                }
            });

        ctx.request_repaint();
        true
    }

    // ─── Titlebar area ─────────────────────────────────────────────────────

    fn render_titlebar(&mut self, ui: &mut egui::Ui) {
        // The 64px gap normally lines the title up next to macOS's native traffic
        // light buttons. Those buttons vanish in fullscreen, so there's no need to
        // clear space for them there — shift the title back left instead of leaving
        // it sitting where the (now-absent) buttons used to be.
        let is_fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        let title_offset = if is_fullscreen { 0.0 } else { 64.0 };

        let titlebar_fill = self.palette().bg_primary;
        let titlebar_text = self.palette().text_primary;
        let accent = self.palette().accent;
        let mcp_active = self.mcp_port != 0;
        egui::Panel::top("titlebar")
            .exact_size(self.cfg.titlebar_height)
            .frame(
                egui::Frame::new()
                    .fill(titlebar_fill)
                    .inner_margin(Margin { left: 12, right: 12, top: 6, bottom: 0 }),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(title_offset);

                    ui.label(
                        egui::RichText::new(&self.manifest.name)
                            .font(FontId::proportional(16.0))
                            .color(titlebar_text)
                            .strong(),
                    );

                    if mcp_active {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let icon_ch = icon_char("globe-check").unwrap_or('○');
                            let btn_resp = egui::Frame::new()
                                .fill(Color32::TRANSPARENT)
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin { left: 8, right: 8, top: 3, bottom: 3 })
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(icon_ch.to_string())
                                                .font(FontId::new(12.0, icon_family()))
                                                .color(accent),
                                        );
                                        ui.label(
                                            egui::RichText::new("Connecteur IA en ligne")
                                                .font(FontId::proportional(11.0))
                                                .color(accent),
                                        );
                                    });
                                })
                                .response
                                .interact(egui::Sense::click());
                            if btn_resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if btn_resp.on_hover_text("Paramètres de connexion").clicked() {
                                self.mcp_modal_open = true;
                            }
                        });
                    }
                });
            });
    }

    fn render_mcp_modal(&mut self, ctx: &egui::Context) {
        if !self.mcp_modal_open {
            return;
        }
        let port = self.mcp_port;
        let url = format!("http://127.0.0.1:{port}/mcp");
        let app_name = self.manifest.name.clone();
        let app_slug = app_name.to_lowercase().replace(' ', "-");

        let screen_rect = ctx.viewport_rect();

        egui::Area::new(egui::Id::new("mcp_modal_overlay"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    Color32::from_rgba_premultiplied(0, 0, 0, 140),
                );
                if response.clicked() {
                    self.mcp_modal_open = false;
                }
            });

        let modal_width = (screen_rect.width() * 0.60).clamp(500.0, 680.0);
        let pal = self.palette().clone();
        let icon_x = icon_char("x").unwrap_or('✕');
        let icon_plug = icon_char("plug").unwrap_or('⚡');
        let icon_terminal = icon_char("terminal").unwrap_or('>');
        let icon_monitor = icon_char("monitor").unwrap_or('□');
        let icon_globe = icon_char("globe").unwrap_or('○');
        let icon_code = icon_char("code").unwrap_or('<');
        let icon_copy = icon_char("copy").unwrap_or('⎘');
        let icon_link = icon_char("link").unwrap_or('🔗');

        egui::Window::new("mcp_setup_modal")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .order(egui::Order::TOP)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([modal_width, (screen_rect.height() * 0.82).clamp(500.0, 720.0)])
            .frame(
                egui::Frame::new()
                    .fill(pal.bg_elevated)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(CornerRadius::same(self.cfg.corner_radius_card as u8))
                    .inner_margin(Margin::same(28))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 12].into(),
                        blur: 40,
                        spread: 4,
                        color: Color32::from_rgba_premultiplied(0, 0, 0, 25),
                    }),
            )
            .show(ctx, |ui| {
                // ── Header ──────────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(icon_plug.to_string())
                            .font(FontId::new(18.0, icon_family()))
                            .color(pal.accent),
                    );
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Connecteur IA")
                                .font(FontId::proportional(17.0))
                                .color(pal.text_primary)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "Connectez un agent IA à {app_name} via MCP."
                            ))
                            .font(FontId::proportional(13.0))
                            .color(pal.text_secondary),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        let close_btn = ui.add(
                            egui::Label::new(
                                egui::RichText::new(icon_x.to_string())
                                    .font(FontId::new(16.0, icon_family()))
                                    .color(pal.text_secondary),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if close_btn.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if close_btn.clicked() {
                            self.mcp_modal_open = false;
                        }
                    });
                });

                ui.add_space(16.0);

                // ── URL badge ────────────────────────────────────────────
                egui::Frame::new()
                    .fill(pal.accent_subtle)
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin { left: 12, right: 12, top: 8, bottom: 8 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(icon_link.to_string())
                                    .font(FontId::new(13.0, icon_family()))
                                    .color(pal.accent),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(&url)
                                    .font(FontId::monospace(12.5))
                                    .color(pal.accent),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let copy_btn = ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(icon_copy.to_string())
                                                .font(FontId::new(14.0, icon_family()))
                                                .color(pal.accent),
                                        )
                                        .sense(egui::Sense::click()),
                                    );
                                    if copy_btn.hovered() {
                                        ui.ctx()
                                            .set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if copy_btn.on_hover_text("Copier l'URL").clicked() {
                                        if let Ok(mut cb) = arboard::Clipboard::new() {
                                            let _ = cb.set_text(&url);
                                        }
                                    }
                                },
                            );
                        });
                    });

                ui.add_space(16.0);

                // ── Instructions scroll area ─────────────────────────────
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mono = FontId::monospace(13.0);

                        // ── Claude Code ──────────────────────────────────
                        mcp_section_header(ui, &pal, icon_terminal, "Claude Code");
                        ui.add_space(4.0);
                        code_block(
                            ui,
                            &pal,
                            &mono,
                            &format!("claude mcp add {app_name} --transport http {url}"),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Ou dans .mcp.json :")
                                .font(FontId::proportional(11.5))
                                .color(pal.text_secondary),
                        );
                        ui.add_space(2.0);
                        code_block(
                            ui,
                            &pal,
                            &mono,
                            &format!(
                                "{{\n  \"mcpServers\": {{\n    \"{app_slug}\": {{\n      \"type\": \"http\",\n      \"url\": \"{url}\"\n    }}\n  }}\n}}"
                            ),
                        );
                        ui.add_space(16.0);

                        // ── Claude Desktop ───────────────────────────────
                        mcp_section_header(ui, &pal, icon_monitor, "Claude Desktop");
                        ui.add_space(4.0);
                        #[cfg(target_os = "macos")]
                        let config_path =
                            "~/Library/Application Support/Claude/claude_desktop_config.json";
                        #[cfg(target_os = "windows")]
                        let config_path =
                            "%APPDATA%\\Claude\\claude_desktop_config.json";
                        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                        let config_path = "~/.config/Claude/claude_desktop_config.json";
                        ui.label(
                            egui::RichText::new(config_path)
                                .font(FontId::monospace(11.0))
                                .color(pal.text_secondary),
                        );
                        ui.add_space(4.0);
                        code_block(
                            ui,
                            &pal,
                            &mono,
                            &format!(
                                "{{\n  \"mcpServers\": {{\n    \"{app_slug}\": {{\n      \"type\": \"http\",\n      \"url\": \"{url}\"\n    }}\n  }}\n}}"
                            ),
                        );
                        ui.add_space(16.0);

                        // ── ChatGPT ──────────────────────────────────────
                        mcp_section_header(ui, &pal, icon_globe, "ChatGPT Desktop");
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(
                                "Settings > Plugins > MCP > Add server",
                            )
                            .font(FontId::proportional(12.0))
                            .color(pal.text_secondary),
                        );
                        ui.add_space(4.0);
                        code_block(ui, &pal, &mono, &url);
                        ui.add_space(16.0);

                        // ── Cursor / VS Code ─────────────────────────────
                        mcp_section_header(ui, &pal, icon_code, "Cursor / VS Code");
                        ui.add_space(4.0);
                        code_block(
                            ui,
                            &pal,
                            &mono,
                            &format!(
                                "{{\n  \"mcp\": {{\n    \"servers\": {{\n      \"{app_slug}\": {{\n        \"type\": \"http\",\n        \"url\": \"{url}\"\n      }}\n    }}\n  }}\n}}"
                            ),
                        );
                        ui.add_space(16.0);

                        // ── Gemini ───────────────────────────────────────
                        mcp_section_header(ui, &pal, icon_globe, "Gemini / AI Studio");
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Extensions → Add an MCP tool (HTTP)")
                                .font(FontId::proportional(12.0))
                                .color(pal.text_secondary),
                        );
                        ui.add_space(4.0);
                        code_block(ui, &pal, &mono, &url);
                    });

                ui.add_space(16.0);

                // ── Footer ───────────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn_text = egui::RichText::new("Fermer")
                            .font(FontId::proportional(13.0))
                            .color(pal.text_secondary);
                        let close_btn =
                            ui.add(egui::Label::new(btn_text).sense(egui::Sense::click()));
                        if close_btn.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if close_btn.clicked() {
                            self.mcp_modal_open = false;
                        }
                    });
                });
            });
    }

    // ─── Node rendering ──────────────────────────────────────────────────────

    fn render_node(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let has_width_hint = node.width.is_some() || node.min_width.is_some() || node.grow;
        if has_width_hint {
            let available_width = ui.available_width();
            let requested_width = if node.grow {
                available_width
            } else {
                node.width.unwrap_or(available_width)
            };
            let width = requested_width
                .max(node.min_width.unwrap_or(0.0))
                .min(available_width);
            let layout = *ui.layout();
            ui.allocate_ui_with_layout(
                Vec2::new(width, ui.available_height()),
                layout,
                |ui| self.render_node_inner(ui, node),
            );
            return;
        }
        self.render_node_inner(ui, node);
    }

    fn render_node_inner(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        if self.hidden_nodes.contains(&node.id) {
            return;
        }
        match node.role.as_str() {
            "screen" => self.render_screen_content(ui, node),
            "dialog" | "view" => {}
            "button" => self.render_button(ui, node),
            "text-input" => self.render_text_input(ui, node),
            "text-area" => self.render_text_area(ui, node),
            "group" => self.render_group(ui, node),
            "label" => self.render_label(ui, node),
            "heading" => self.render_heading(ui, node),
            "checkbox" => self.render_checkbox(ui, node),
            "toggle" => self.render_toggle(ui, node),
            "select" => self.render_select(ui, node),
            "badge" => self.render_badge(ui, node),
            "separator" => self.render_separator(ui, node),
            "list" => self.render_list(ui, node),
            "list-item" => self.render_list_item(ui, node),
            "progress" => self.render_progress(ui, node),
            "slider" => self.render_slider(ui, node),
            "chip" => self.render_chip(ui, node),
            "image" => self.render_image_placeholder(ui, node),
            "card" => self.render_card(ui, node),
            "option" => {}
            _ => {
                ui.label(egui::RichText::new(&node.label).color(self.c_text_primary()));
                for child in node.children.clone() {
                    self.render_node(ui, &child);
                }
            }
        }
    }

    fn render_children_with_layout(
        &mut self,
        ui: &mut egui::Ui,
        node: &SemanticNode,
        children: &[SemanticNode],
        default_layout: &str,
        default_gap: f32,
    ) {
        if children.is_empty() {
            return;
        }

        let layout = node.layout.as_deref().unwrap_or(default_layout);
        let gap = node.gap.unwrap_or(default_gap);

        match layout {
            "row" => {
                if node.wrap {
                    ui.horizontal_wrapped(|ui| {
                        for (idx, child) in children.iter().enumerate() {
                            self.render_node(ui, child);
                            if idx < children.len() - 1 {
                                ui.add_space(gap);
                            }
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        for (idx, child) in children.iter().enumerate() {
                            self.render_node(ui, child);
                            if idx < children.len() - 1 {
                                ui.add_space(gap);
                            }
                        }
                    });
                }
            }
            "grid" => {
                let columns = node.columns.unwrap_or(2).max(1);
                egui::Grid::new(format!("grid_{}", node.id))
                    .num_columns(columns)
                    .spacing(Vec2::new(gap, gap))
                    .striped(false)
                    .show(ui, |ui| {
                        for (idx, child) in children.iter().enumerate() {
                            self.render_node(ui, child);
                            if (idx + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                        if children.len() % columns != 0 {
                            ui.end_row();
                        }
                    });
            }
            _ => {
                for (idx, child) in children.iter().enumerate() {
                    self.render_node(ui, child);
                    if idx < children.len() - 1 {
                        ui.add_space(gap);
                    }
                }
            }
        }
    }

    fn render_screen_content(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let children = node.children.clone();
        let visible_children: Vec<_> = children
            .iter()
            .filter(|c| c.role != "dialog" && c.role != "view")
            .collect();

        if matches!(node.layout.as_deref(), Some("row") | Some("wrap") | Some("grid")) {
            let visible_children: Vec<SemanticNode> = visible_children.into_iter().cloned().collect();
            self.render_children_with_layout(ui, node, &visible_children, "column", node.gap.unwrap_or(12.0));
            return;
        }

        for (idx, child) in visible_children.iter().enumerate() {
            let appear = self
                .node_appear_anims
                .entry(child.id.clone())
                .or_insert_with(|| NodeAppearAnim {
                    first_seen: Instant::now(),
                });

            let stagger_delay = idx as f32 * 0.05;
            let elapsed = appear.first_seen.elapsed().as_secs_f32() - stagger_delay;
            let appear_duration = 0.35;

            let (opacity, offset_y) = if elapsed < 0.0 {
                (0.0, 8.0)
            } else if elapsed < appear_duration {
                let t = elapsed / appear_duration;
                (ease_out_quart(t), (1.0 - ease_out_quart(t)) * 8.0)
            } else {
                (1.0, 0.0)
            };

            if opacity < 1.0 {
                ui.ctx().request_repaint();
            }

            if opacity > 0.001 {
                ui.set_opacity(opacity);
                ui.add_space(offset_y);
                self.render_node(ui, child);
                ui.add_space(node.gap.unwrap_or(12.0));
                ui.set_opacity(1.0);
            }
        }
    }

    fn render_group(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let available = ui.available_width();
        let group_id = egui::Id::new(format!("group_hover_{}", node.id));

        let pre_response = ui.allocate_rect(
            egui::Rect::from_min_size(ui.cursor().min, Vec2::new(available, 0.0)),
            egui::Sense::hover(),
        );
        let is_hovered = pre_response.hovered();
        let hover_t = ui.ctx().animate_bool_with_time(group_id, is_hovered, 0.25);

        let border_color = lerp_color(self.c_border(), Color32::from_rgb(0xD0, 0xD5, 0xE0), hover_t);
        let shadow_alpha = (hover_t * 8.0) as u8;

        let frame = egui::Frame::new()
            .fill(self.c_bg_surface())
            .stroke(Stroke::new(1.0, border_color))
            .corner_radius(CornerRadius::same(self.cfg.corner_radius_card as u8))
            .inner_margin(Margin::same(node.padding.unwrap_or(20.0) as i8))
            .shadow(egui::epaint::Shadow {
                offset: [0, (hover_t * 2.0) as i8].into(),
                blur: (hover_t * 6.0) as u8,
                spread: 0,
                color: Color32::from_rgba_premultiplied(0, 0, 0, shadow_alpha),
            });
        frame.show(ui, |ui| {
            if !node.label.is_empty() {
                ui.label(
                    egui::RichText::new(&node.label)
                        .font(FontId::proportional(15.0))
                        .color(self.c_text_primary())
                        .strong(),
                );
                ui.add_space(node.gap.unwrap_or(12.0));
            }
            let children = node.children.clone();
            self.render_children_with_layout(ui, node, &children, "column", 8.0);
        });
    }

    fn render_text_input(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        if let Some(bind_key) = &node.bind.clone() {
            let disabled = node.disabled;
            let readonly = node.readonly;
            let placeholder = node.placeholder.clone().unwrap_or_default();
            ui.add_enabled_ui(!disabled, |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(&node.label)
                            .font(FontId::proportional(13.0))
                            .color(self.c_text_secondary())
                            .strong(),
                    );
                    ui.add_space(6.0);
                    let value = self.state.entry(bind_key.clone()).or_default();
                    let field_width = ui.available_width();
                    ui.add(
                        egui::TextEdit::singleline(value)
                            .desired_width(field_width)
                            .margin(Vec2::new(14.0, 11.0))
                            .font(FontId::proportional(14.0))
                            .hint_text(&placeholder)
                            .interactive(!readonly),
                    );
                });
            });
        }
    }

    fn render_text_area(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        if let Some(bind_key) = &node.bind.clone() {
            let disabled = node.disabled;
            let readonly = node.readonly;
            let placeholder = node.placeholder.clone().unwrap_or_default();
            ui.add_enabled_ui(!disabled, |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(&node.label)
                            .font(FontId::proportional(13.0))
                            .color(self.c_text_secondary())
                            .strong(),
                    );
                    ui.add_space(6.0);
                    let value = self.state.entry(bind_key.clone()).or_default();
                    let field_width = ui.available_width();
                    ui.add(
                        egui::TextEdit::multiline(value)
                            .desired_width(field_width)
                            .desired_rows(5)
                            .margin(Vec2::new(14.0, 11.0))
                            .font(FontId::proportional(14.0))
                            .hint_text(&placeholder)
                            .interactive(!readonly),
                    );
                });
            });
        }
    }

    fn render_button(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let actions = node.actions.clone();
        let node_clone = node.clone();

        for action in &actions {
            if !action.parameters.is_empty() {
                let param_frame = egui::Frame::new()
                    .fill(self.c_bg_surface())
                    .stroke(Stroke::new(1.0, self.c_border()))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(14));
                param_frame.show(ui, |ui| {
                    for parameter in &action.parameters {
                        let key = form_key(&node_clone.id, &action.id, &parameter.name);
                        ui.vertical(|ui| {
                            let label_text = if parameter.description.is_empty() {
                                &parameter.name
                            } else {
                                &parameter.description
                            };
                            let suffix = if parameter.required { " *" } else { "" };
                            ui.label(
                                egui::RichText::new(format!("{label_text}{suffix}"))
                                    .font(FontId::proportional(12.0))
                                    .color(self.c_text_secondary()),
                            );
                            ui.add_space(4.0);
                            let field_width = ui.available_width();
                            ui.add(
                                egui::TextEdit::singleline(
                                    self.form_values.entry(key).or_default(),
                                )
                                .desired_width(field_width)
                                .margin(Vec2::new(12.0, 9.0))
                                .font(FontId::proportional(14.0)),
                            );
                        });
                        ui.add_space(8.0);
                    }
                });
                ui.add_space(10.0);
            }

            let icon = node_clone.icon.as_deref().and_then(icon_char);
            let btn_id = format!("{}_{}", node_clone.id, action.id);
            let anim = self.button_anims.entry(btn_id).or_default().clone();

            let (btn_accent, btn_accent_hover) = match node_clone.variant.as_deref() {
                Some("secondary") => (self.c_bg_surface(), self.c_border()),
                Some("destructive") => (self.c_error_text(), Color32::from_rgb(0xB9, 0x1C, 0x1C)),
                _ => (self.c_accent(), self.c_accent_hover()),
            };
            let btn_text_color = match node_clone.variant.as_deref() {
                Some("secondary") => self.c_text_primary(),
                _ => self.c_text_on_accent(),
            };

            let disabled = node_clone.disabled;
            ui.add_enabled_ui(!disabled, |ui| {
                if animated_primary_button(ui, &node_clone.label, icon, &anim, btn_accent, btn_accent_hover, btn_text_color, self.cfg.corner_radius_button as u8).clicked() {
                    self.button_anims
                        .get_mut(&format!("{}_{}", node_clone.id, action.id))
                        .unwrap()
                        .press_start = Some(Instant::now());
                    self.invoke_action(&node_clone, action);
                }
            });

            let dt = ui.input(|i| i.stable_dt);
            let btn_key = format!("{}_{}", node_clone.id, action.id);
            if let Some(ba) = self.button_anims.get_mut(&btn_key) {
                if ba.press_start.is_some() {
                    ui.ctx().request_repaint();
                }
                let _ = dt;
            }
        }

        let children = node.children.clone();
        for child in &children {
            self.render_node(ui, child);
        }
    }

    // ─── New components ────────────────────────────────────────────────────────

    fn render_label(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let text = if let Some(bind_key) = &node.bind {
            self.state.get(bind_key).cloned().unwrap_or_default()
        } else {
            node.label.clone()
        };
        ui.horizontal(|ui| {
            if let Some(icon_ch) = node.icon.as_deref().and_then(icon_char) {
                ui.label(
                    egui::RichText::new(icon_ch.to_string())
                        .font(FontId::new(14.0, icon_family()))
                        .color(self.c_text_secondary()),
                );
                ui.add_space(4.0);
            }
            ui.label(
                egui::RichText::new(text)
                    .font(FontId::proportional(14.0))
                    .color(self.c_text_primary()),
            );
        });
    }

    fn render_heading(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        ui.horizontal(|ui| {
            if let Some(icon_ch) = node.icon.as_deref().and_then(icon_char) {
                ui.label(
                    egui::RichText::new(icon_ch.to_string())
                        .font(FontId::new(18.0, icon_family()))
                        .color(self.c_text_primary()),
                );
                ui.add_space(6.0);
            }
            ui.label(
                egui::RichText::new(&node.label)
                    .font(FontId::proportional(18.0))
                    .color(self.c_text_primary())
                    .strong(),
            );
        });
    }

    fn render_checkbox(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        if let Some(bind_key) = &node.bind.clone() {
            let current = self.state.get(bind_key).map(|s| s == "true").unwrap_or(false);
            let mut checked = current;
            let disabled = node.disabled;

            let anim = self.checkbox_anims.entry(node.id.clone()).or_default();
            let target_t = if current { 1.0 } else { 0.0 };
            let dt = ui.input(|i| i.stable_dt);
            let speed = if target_t > anim.check_t { 9.0 } else { 12.0 };
            if (anim.check_t - target_t).abs() > 0.001 {
                anim.check_t += (target_t - anim.check_t) * (speed * dt).min(1.0);
                ui.ctx().request_repaint();
            } else {
                anim.check_t = target_t;
            }
            let check_t = anim.check_t.clamp(0.0, 1.0);

            ui.add_enabled_ui(!disabled, |ui| {
                ui.horizontal(|ui| {
                    let box_size = 18.0;
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(box_size, box_size), egui::Sense::click());

                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if response.clicked() {
                        checked = !checked;
                    }

                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();
                        let eased = ease_out_cubic(check_t);

                        let bg = lerp_color(self.cfg.input_bg, self.c_accent(), eased);
                        let border_color = lerp_color(self.cfg.input_border, self.c_accent(), eased);
                        let press_scale = if response.is_pointer_button_down_on() {
                            0.9
                        } else if response.hovered() {
                            1.05
                        } else {
                            1.0
                        };
                        let radius = (box_size / 2.0) * press_scale;

                        painter.circle_filled(rect.center(), radius, bg);
                        painter.circle_stroke(rect.center(), radius, Stroke::new(1.5, border_color));

                        // Animated checkmark: pops in with a slight overshoot, fades with the box.
                        if check_t > 0.001 {
                            let pop = ease_out_back(check_t).clamp(0.0, 1.15);
                            let alpha = (check_t * 255.0) as u8;
                            let c = rect.center();
                            let p1 = c + Vec2::new(-4.0, 0.5) * pop;
                            let p2 = c + Vec2::new(-1.2, 3.2) * pop;
                            let p3 = c + Vec2::new(4.2, -3.4) * pop;
                            let stroke = Stroke::new(
                                2.0,
                                Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
                            );
                            painter.line_segment([p1, p2], stroke);
                            painter.line_segment([p2, p3], stroke);
                        }
                    }

                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(&node.label)
                            .font(FontId::proportional(14.0))
                            .color(self.c_text_primary()),
                    );
                });
            });

            if checked != current {
                self.state.insert(bind_key.clone(), checked.to_string());
            }
        } else {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(false, |ui| {
                    let mut checked = false;
                    ui.checkbox(&mut checked, &node.label);
                });
            });
        }
    }

    fn render_toggle(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        if let Some(bind_key) = &node.bind.clone() {
            let current = self.state.get(bind_key).map(|s| s == "true").unwrap_or(false);
            let mut on = current;
            let disabled = node.disabled;

            let anim = self.toggle_anims.entry(node.id.clone()).or_default();
            let target_t = if current { 1.0 } else { 0.0 };
            let dt = ui.input(|i| i.stable_dt);
            let speed = 6.0;
            if (anim.position_t - target_t).abs() > 0.001 {
                anim.position_t += (target_t - anim.position_t) * (speed * dt).min(1.0);
                ui.ctx().request_repaint();
            } else {
                anim.position_t = target_t;
            }
            let position_t = anim.position_t;

            ui.add_enabled_ui(!disabled, |ui| {
            ui.horizontal(|ui| {
                let size = Vec2::new(44.0, 24.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked() {
                    on = !on;
                }
                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();
                    let bg = lerp_color(self.cfg.input_border, self.c_accent(), ease_out_cubic(position_t));
                    painter.rect_filled(rect, CornerRadius::same(12), bg);

                    let left_x = rect.left() + 12.0;
                    let right_x = rect.right() - 12.0;
                    let circle_x = left_x + (right_x - left_x) * ease_out_back(position_t);

                    let scale = if response.is_pointer_button_down_on() {
                        0.85
                    } else if response.hovered() {
                        1.05
                    } else {
                        1.0
                    };
                    painter.circle_filled(
                        egui::pos2(circle_x, rect.center().y),
                        9.0 * scale,
                        Color32::WHITE,
                    );
                }
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(&node.label)
                        .font(FontId::proportional(14.0))
                        .color(self.c_text_primary()),
                );
            });
            }); // add_enabled_ui

            if on != current {
                self.state.insert(bind_key.clone(), on.to_string());
            }
        }
    }

    fn render_select(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        if let Some(bind_key) = &node.bind.clone() {
            let options: Vec<String> = node
                .children
                .iter()
                .filter(|c| c.role == "option")
                .map(|c| c.label.clone())
                .collect();

            let current = self.state.entry(bind_key.clone()).or_default().clone();
            let display_text = if current.is_empty() {
                "Select...".to_string()
            } else {
                current.clone()
            };

            let select_key = node.id.clone();
            let is_open = self.open_selects.contains(&select_key);
            let disabled = node.disabled;

            ui.add_enabled_ui(!disabled, |ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(&node.label)
                        .font(FontId::proportional(13.0))
                        .color(self.c_text_secondary())
                        .strong(),
                );
                ui.add_space(6.0);

                // Trigger button
                let desired_width = ui.available_width().min(280.0);
                let btn_size = Vec2::new(desired_width, 38.0);
                let (btn_rect, btn_response) = ui.allocate_exact_size(btn_size, egui::Sense::click());

                if btn_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                if btn_response.clicked() {
                    if is_open {
                        // Start close animation instead of instant removal
                        let a = self.select_anims.entry(select_key.clone()).or_default();
                        if a.closing_at.is_none() {
                            a.closing_at = Some(Instant::now());
                        }
                    } else {
                        self.open_selects.insert(select_key.clone());
                        let a = self.select_anims.entry(select_key.clone()).or_default();
                        a.opened_at = Some(Instant::now());
                        a.closing_at = None;
                        a.selected_flash = None;
                    }
                }

                let hover_id = egui::Id::new(format!("select_hover_{}", node.id));
                let hover_t = ui.ctx().animate_bool_with_time(hover_id, btn_response.hovered() || is_open, 0.15);

                if ui.is_rect_visible(btn_rect) {
                    let painter = ui.painter();

                    let border_color = lerp_color(self.cfg.input_border, self.c_accent(), hover_t * 0.6);
                    let bg = lerp_color(self.cfg.input_bg, Color32::from_rgb(0xF8, 0xFA, 0xFF), hover_t);
                    let stroke = if is_open {
                        Stroke::new(1.5, self.c_accent())
                    } else {
                        Stroke::new(1.0, border_color)
                    };

                    painter.rect(btn_rect, CornerRadius::same(8), bg, stroke, egui::StrokeKind::Inside);

                    let text_color = if current.is_empty() { self.c_text_secondary() } else { self.c_text_primary() };
                    painter.text(
                        egui::pos2(btn_rect.left() + 14.0, btn_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &display_text,
                        FontId::proportional(14.0),
                        text_color,
                    );

                    // Animated chevron (drawn as two lines, rotated when open)
                    let chevron_center = egui::pos2(btn_rect.right() - 20.0, btn_rect.center().y);
                    let open_t = ui.ctx().animate_bool_with_time(
                        egui::Id::new(format!("select_chevron_{}", node.id)),
                        is_open,
                        0.25,
                    );
                    let angle = ease_out_cubic(open_t) * std::f32::consts::FRAC_PI_2;
                    let half = 4.5_f32;
                    let rotate = |dx: f32, dy: f32| -> egui::Pos2 {
                        let cos = angle.cos();
                        let sin = angle.sin();
                        egui::pos2(
                            chevron_center.x + dx * cos - dy * sin,
                            chevron_center.y + dx * sin + dy * cos,
                        )
                    };
                    let color = lerp_color(self.c_text_secondary(), self.c_accent(), hover_t);
                    let stroke = Stroke::new(1.8, color);
                    painter.line_segment([rotate(-half, -half * 0.5), rotate(0.0, half * 0.5)], stroke);
                    painter.line_segment([rotate(0.0, half * 0.5), rotate(half, -half * 0.5)], stroke);
                }

                // Popup dropdown with open/close/select animations
                let anim = self.select_anims.entry(select_key.clone()).or_default();

                // Handle open/close state transitions
                if is_open && anim.opened_at.is_none() && anim.closing_at.is_none() {
                    anim.opened_at = Some(Instant::now());
                }

                // Finish close animation
                let mut should_fully_close = false;
                if let Some(close_start) = anim.closing_at {
                    if close_start.elapsed().as_secs_f32() >= SELECT_CLOSE_DURATION {
                        should_fully_close = true;
                    }
                }
                if should_fully_close {
                    self.open_selects.remove(&select_key);
                    let a = self.select_anims.get_mut(&select_key).unwrap();
                    a.opened_at = None;
                    a.closing_at = None;
                    a.selected_flash = None;
                }

                let anim = self.select_anims.get(&select_key).cloned().unwrap_or_default();
                let show_popup = anim.opened_at.is_some();

                if show_popup {
                    let open_elapsed = anim.opened_at.unwrap().elapsed().as_secs_f32();
                    let is_closing = anim.closing_at.is_some();
                    let close_elapsed = anim.closing_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);

                    let container_t = if is_closing {
                        let t = (close_elapsed / SELECT_CLOSE_DURATION).min(1.0);
                        1.0 - ease_out_cubic(t)
                    } else {
                        let t = (open_elapsed / SELECT_OPEN_DURATION).min(1.0);
                        ease_out_quart(t)
                    };

                    let container_alpha = (container_t * 255.0) as u8;
                    let shadow_alpha = (container_t * 25.0) as u8;
                    let offset_y = (1.0 - container_t) * 8.0;

                    let popup_id = egui::Id::new(format!("select_popup_{}", node.id));
                    let area_response = egui::Area::new(popup_id)
                        .order(egui::Order::Tooltip)
                        .fixed_pos(egui::pos2(btn_rect.left(), btn_rect.bottom() + 4.0 + offset_y))
                        .show(ui.ctx(), |ui| {
                            let frame = egui::Frame::new()
                                .fill(Color32::from_rgba_unmultiplied(
                                    self.c_bg_surface().r(), self.c_bg_surface().g(), self.c_bg_surface().b(), container_alpha,
                                ))
                                .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(
                                    self.c_border().r(), self.c_border().g(), self.c_border().b(), container_alpha,
                                )))
                                .corner_radius(CornerRadius::same(10))
                                .inner_margin(Margin { left: 4, right: 4, top: 6, bottom: 6 })
                                .shadow(egui::epaint::Shadow {
                                    offset: [0, (4.0 * container_t) as i8].into(),
                                    blur: (16.0 * container_t) as u8,
                                    spread: (2.0 * container_t) as u8,
                                    color: Color32::from_rgba_premultiplied(0, 0, 0, shadow_alpha),
                                });
                            frame.show(ui, |ui| {
                                ui.set_width(desired_width - 8.0);
                                for (opt_idx, option) in options.iter().enumerate() {
                                    let item_progress = if is_closing {
                                        // Reverse stagger on close (last items disappear first)
                                        let reverse_idx = (options.len() - 1 - opt_idx) as f32;
                                        let item_delay = reverse_idx * SELECT_ITEM_STAGGER * 0.5;
                                        let item_t = ((close_elapsed - item_delay) / (SELECT_CLOSE_DURATION * 0.7)).clamp(0.0, 1.0);
                                        1.0 - ease_out_cubic(item_t)
                                    } else {
                                        let item_delay = opt_idx as f32 * SELECT_ITEM_STAGGER;
                                        let item_elapsed = (open_elapsed - item_delay).max(0.0);
                                        let item_duration = SELECT_OPEN_DURATION * 0.8;
                                        let item_t = (item_elapsed / item_duration).min(1.0);
                                        ease_out_cubic(item_t)
                                    };

                                    let is_selected = *option == current;
                                    let opt_size = Vec2::new(desired_width - 8.0, 34.0);
                                    let (opt_rect, opt_response) =
                                        ui.allocate_exact_size(opt_size, egui::Sense::click());

                                    if opt_response.hovered() && !is_closing {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }

                                    if ui.is_rect_visible(opt_rect) && item_progress > 0.001 {
                                        let item_alpha = (item_progress * 255.0) as u8;
                                        let item_offset_y = (1.0 - item_progress) * 10.0;
                                        let rendered_rect = opt_rect.translate(Vec2::new(0.0, item_offset_y));

                                        // Flash animation on recently selected item
                                        let flash_t = anim.selected_flash.as_ref()
                                            .filter(|(name, _)| name == option)
                                            .map(|(_, start)| {
                                                let t = start.elapsed().as_secs_f32() / SELECT_FLASH_DURATION;
                                                if t < 0.5 { ease_out_cubic(t * 2.0) } else { 1.0 - ease_out_cubic((t - 0.5) * 2.0) }
                                            })
                                            .unwrap_or(0.0);

                                        let opt_hover_id = egui::Id::new(format!("opt_{}_{}", node.id, option));
                                        let opt_hover_t = ui.ctx().animate_bool_with_time(
                                            opt_hover_id,
                                            opt_response.hovered() && !is_closing,
                                            0.15,
                                        );

                                        let opt_bg = if flash_t > 0.001 {
                                            lerp_color(
                                                if is_selected { self.c_accent_subtle() } else { Color32::TRANSPARENT },
                                                Color32::from_rgb(0xDB, 0xE8, 0xFD),
                                                flash_t,
                                            )
                                        } else if is_selected {
                                            Color32::from_rgba_unmultiplied(
                                                self.c_accent_subtle().r(), self.c_accent_subtle().g(), self.c_accent_subtle().b(), item_alpha,
                                            )
                                        } else {
                                            Color32::from_rgba_unmultiplied(
                                                0xF5, 0xF6, 0xF8, (opt_hover_t * item_progress * 255.0) as u8,
                                            )
                                        };

                                        ui.painter().rect_filled(
                                            rendered_rect,
                                            CornerRadius::same(6),
                                            opt_bg,
                                        );

                                        let text_color = if is_selected {
                                            Color32::from_rgba_unmultiplied(self.c_accent().r(), self.c_accent().g(), self.c_accent().b(), item_alpha)
                                        } else {
                                            Color32::from_rgba_unmultiplied(self.c_text_primary().r(), self.c_text_primary().g(), self.c_text_primary().b(), item_alpha)
                                        };
                                        ui.painter().text(
                                            egui::pos2(rendered_rect.left() + 12.0, rendered_rect.center().y),
                                            egui::Align2::LEFT_CENTER,
                                            option,
                                            FontId::proportional(14.0),
                                            text_color,
                                        );

                                        if is_selected {
                                            ui.painter().text(
                                                egui::pos2(rendered_rect.right() - 12.0, rendered_rect.center().y),
                                                egui::Align2::RIGHT_CENTER,
                                                "\u{e06c}",
                                                FontId::new(13.0, icon_family()),
                                                Color32::from_rgba_unmultiplied(self.c_accent().r(), self.c_accent().g(), self.c_accent().b(), item_alpha),
                                            );
                                        }
                                    }

                                    if opt_response.clicked() && !is_closing {
                                        self.state.insert(bind_key.clone(), option.clone());
                                        if let Some(a) = self.select_anims.get_mut(&select_key) {
                                            a.selected_flash = Some((option.clone(), Instant::now()));
                                            a.closing_at = Some(Instant::now());
                                        }
                                    }
                                }
                            });
                        });

                    ui.ctx().request_repaint();

                    // Close on click outside (start close animation)
                    if !is_closing && btn_response.clicked_elsewhere() && area_response.response.clicked_elsewhere() {
                        if let Some(a) = self.select_anims.get_mut(&select_key) {
                            a.closing_at = Some(Instant::now());
                        }
                    }
                }
            });
            }); // add_enabled_ui
        }
    }

    fn render_badge(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let text = if let Some(bind_key) = &node.bind {
            self.state.get(bind_key).cloned().unwrap_or_default()
        } else {
            node.label.clone()
        };

        let badge_id = egui::Id::new(format!("badge_{}", node.id));
        let response = ui.allocate_rect(
            egui::Rect::from_min_size(ui.cursor().min, Vec2::new(0.0, 0.0)),
            egui::Sense::hover(),
        );
        let hover_t = ui.ctx().animate_bool_with_time(badge_id, response.hovered(), 0.2);
        let fill = lerp_color(self.c_accent_subtle(), Color32::from_rgb(0xDB, 0xE4, 0xFD), hover_t);

        let frame = egui::Frame::new()
            .fill(fill)
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin { left: 10, right: 10, top: 4, bottom: 4 });
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(icon_ch) = node.icon.as_deref().and_then(icon_char) {
                    ui.label(
                        egui::RichText::new(icon_ch.to_string())
                            .font(FontId::new(12.0, icon_family()))
                            .color(self.c_accent()),
                    );
                    ui.add_space(4.0);
                }
                ui.label(
                    egui::RichText::new(text)
                        .font(FontId::proportional(12.0))
                        .color(self.c_accent())
                        .strong(),
                );
            });
        });
    }

    fn render_separator(&mut self, ui: &mut egui::Ui, _node: &SemanticNode) {
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
    }

    fn render_list(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let frame = egui::Frame::new()
            .fill(self.c_bg_surface())
            .stroke(Stroke::new(1.0, self.c_border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin { left: 4, right: 4, top: 8, bottom: 8 });
        frame.show(ui, |ui| {
            if !node.label.is_empty() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(&node.label)
                            .font(FontId::proportional(13.0))
                            .color(self.c_text_secondary())
                            .strong(),
                    );
                });
                ui.add_space(2.0);
            }
            let children = node.children.clone();
            let needs_scroll = node.scroll || children.len() > 6;
            let render_items = |ui: &mut egui::Ui, this: &mut Self| {
                if matches!(node.layout.as_deref(), Some("row") | Some("wrap") | Some("grid")) {
                    this.render_children_with_layout(ui, node, &children, "column", node.gap.unwrap_or(0.0));
                } else {
                    for (idx, child) in children.iter().enumerate() {
                        this.render_node(ui, child);
                        if idx < children.len() - 1 {
                            ui.separator();
                        }
                    }
                }
            };
            if needs_scroll {
                egui::ScrollArea::vertical()
                    .max_height(node.max_height.unwrap_or(280.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        render_items(ui, self);
                    });
            } else {
                render_items(ui, self);
            }
        });
    }

    fn render_list_item(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let node_clone = node.clone();
        let has_actions = !node.actions.is_empty();

        let sense = if has_actions {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), sense);

        if has_actions && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let dt = ui.input(|i| i.stable_dt);
        let anim = self.list_item_anims.entry(node.id.clone()).or_default();
        let target = if has_actions && response.hovered() { 1.0 } else { 0.0 };
        let speed = if target > anim.hover_t { 10.0 } else { 6.0 };
        if (anim.hover_t - target).abs() > 0.001 {
            anim.hover_t += (target - anim.hover_t) * (speed * dt).min(1.0);
            ui.ctx().request_repaint();
        } else {
            anim.hover_t = target;
        }
        let hover_t = anim.hover_t;

        if ui.is_rect_visible(rect) {
            if hover_t > 0.001 {
                let bg = Color32::from_rgba_unmultiplied(
                    self.c_accent_subtle().r(),
                    self.c_accent_subtle().g(),
                    self.c_accent_subtle().b(),
                    (hover_t * 255.0) as u8,
                );
                ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
            }

            let translate_x = hover_t * 3.0;
            let mut x = rect.left() + 12.0 + translate_x;
            if let Some(icon_ch) = node_clone.icon.as_deref().and_then(icon_char) {
                let icon_color = lerp_color(self.c_text_secondary(), self.c_accent(), hover_t);
                ui.painter().text(
                    egui::pos2(x + 8.0, rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    icon_ch.to_string(),
                    FontId::new(14.0, icon_family()),
                    icon_color,
                );
                x += 24.0;
            }

            let text_color = lerp_color(self.c_text_primary(), self.cfg.sidebar_text_active, hover_t * 0.3);
            ui.painter().text(
                egui::pos2(x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &node_clone.label,
                FontId::proportional(14.0),
                text_color,
            );
        }

        if has_actions && response.clicked() {
            let actions = node.actions.clone();
            let node_clone2 = node.clone();
            self.invoke_action(&node_clone2, &actions[0]);
        }
    }

    fn render_progress(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let value = if let Some(bind_key) = &node.bind {
            self.state
                .get(bind_key)
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(0.0)
                .clamp(0.0, 100.0)
                / 100.0
        } else {
            0.5
        };

        let progress_id = egui::Id::new(format!("progress_{}", node.id));
        let animated_value = ui.ctx().animate_value_with_time(progress_id, value, 0.6);

        ui.vertical(|ui| {
            if !node.label.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&node.label)
                            .font(FontId::proportional(13.0))
                            .color(self.c_text_secondary()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{}%", (animated_value * 100.0) as u32))
                                .font(FontId::proportional(12.0))
                                .color(self.c_text_secondary()),
                        );
                    });
                });
                ui.add_space(6.0);
            }
            let bar_width = ui.available_width();
            ui.add(
                egui::ProgressBar::new(animated_value)
                    .desired_width(bar_width)
                    .corner_radius(CornerRadius::same(4)),
            );
        });
    }

    fn render_slider(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        if let Some(bind_key) = &node.bind.clone() {
            let current: f64 = self
                .state
                .get(bind_key)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let mut val = current;

            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(&node.label)
                        .font(FontId::proportional(13.0))
                        .color(self.c_text_secondary())
                        .strong(),
                );
                ui.add_space(6.0);
                ui.add(egui::Slider::new(&mut val, 0.0..=100.0).show_value(true));
            });

            if val != current {
                self.state.insert(bind_key.clone(), val.to_string());
            }
        }
    }

    fn render_chip(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let chip_id = egui::Id::new(format!("chip_{}", node.id));
        let response = ui.allocate_rect(
            egui::Rect::from_min_size(ui.cursor().min, Vec2::new(0.0, 0.0)),
            egui::Sense::hover(),
        );
        let hover_t = ui.ctx().animate_bool_with_time(chip_id, response.hovered(), 0.2);

        let fill = lerp_color(self.cfg.sidebar_bg, Color32::from_rgb(0xE8, 0xEB, 0xF0), hover_t);
        let border_color = lerp_color(self.c_border(), Color32::from_rgb(0xC0, 0xC5, 0xD0), hover_t);

        let frame = egui::Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0, border_color))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin { left: 12, right: 12, top: 6, bottom: 6 });
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(icon_ch) = node.icon.as_deref().and_then(icon_char) {
                    let icon_color = lerp_color(self.c_text_secondary(), self.c_accent(), hover_t);
                    ui.label(
                        egui::RichText::new(icon_ch.to_string())
                            .font(FontId::new(12.0, icon_family()))
                            .color(icon_color),
                    );
                    ui.add_space(4.0);
                }
                ui.label(
                    egui::RichText::new(&node.label)
                        .font(FontId::proportional(13.0))
                        .color(self.c_text_primary()),
                );
            });
        });
    }

    fn render_image_placeholder(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let size = Vec2::new(ui.available_width().min(320.0), 180.0);
        let frame = egui::Frame::new()
            .fill(self.cfg.sidebar_bg)
            .stroke(Stroke::new(1.0, self.c_border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(0));
        frame.show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter();
            let icon_ch = node.icon.as_deref().and_then(icon_char).unwrap_or('\u{e0c0}');
            painter.text(
                rect.center() - Vec2::new(0.0, 10.0),
                egui::Align2::CENTER_CENTER,
                icon_ch.to_string(),
                FontId::new(32.0, icon_family()),
                self.c_text_secondary(),
            );
            painter.text(
                rect.center() + Vec2::new(0.0, 16.0),
                egui::Align2::CENTER_CENTER,
                &node.label,
                FontId::proportional(12.0),
                self.c_text_secondary(),
            );
        });
    }

    fn render_card(&mut self, ui: &mut egui::Ui, node: &SemanticNode) {
        let frame = egui::Frame::new()
            .fill(self.c_bg_surface())
            .stroke(Stroke::new(1.0, self.c_border()))
            .corner_radius(CornerRadius::same(self.cfg.corner_radius_card as u8))
            .inner_margin(Margin::same(node.padding.unwrap_or(20.0) as i8))
            .shadow(egui::epaint::Shadow {
                offset: [0, 2].into(),
                blur: 8,
                spread: 0,
                color: Color32::from_rgba_premultiplied(0, 0, 0, 10),
            });
        frame.show(ui, |ui| {
            if !node.label.is_empty() {
                ui.horizontal(|ui| {
                    if let Some(icon_ch) = node.icon.as_deref().and_then(icon_char) {
                        ui.label(
                            egui::RichText::new(icon_ch.to_string())
                                .font(FontId::new(18.0, icon_family()))
                                .color(self.c_accent()),
                        );
                        ui.add_space(8.0);
                    }
                    ui.label(
                        egui::RichText::new(&node.label)
                            .font(FontId::proportional(15.0))
                            .color(self.c_text_primary())
                            .strong(),
                    );
                });
            }
            if !node.children.is_empty() {
                ui.add_space(node.gap.unwrap_or(12.0));
                let children = node.children.clone();
                self.render_children_with_layout(ui, node, &children, "column", 8.0);
            }
        });
    }

    // ─── Modal dialogs ───────────────────────────────────────────────────────

    fn render_dialogs(&mut self, ctx: &egui::Context) {
        // Clean up finished close animations FIRST
        let mut to_remove = Vec::new();
        for (id, anim) in &self.dialog_anims {
            if anim.closing {
                if let Some(close_start) = anim.close_start {
                    if close_start.elapsed().as_secs_f32() >= DIALOG_CLOSE_DURATION {
                        to_remove.push(id.clone());
                    }
                }
            }
        }
        for id in &to_remove {
            self.dialog_anims.remove(id);
            self.open_views.remove(id);
        }

        let screens = self.screens().into_iter().cloned().collect::<Vec<_>>();
        let dialogs: Vec<SemanticNode> = screens
            .iter()
            .flat_map(|s| s.children.iter())
            .filter(|n| (n.role == "dialog" || n.role == "view") && self.open_views.contains(&n.id))
            .cloned()
            .collect();

        if dialogs.is_empty() {
            return;
        }

        let screen_rect = ctx.viewport_rect();

        for dialog in &dialogs {
            let dialog_id = dialog.id.clone();
            let dialog_label = dialog.label.clone();

            let anim = self.dialog_anims.get(&dialog_id).cloned();
            let (progress, is_closing) = if let Some(ref a) = anim {
                if a.closing {
                    let t = a.close_start.map(|s| s.elapsed().as_secs_f32() / DIALOG_CLOSE_DURATION).unwrap_or(1.0).min(1.0);
                    (1.0 - ease_out_cubic(t), true)
                } else {
                    let t = (a.open_start.elapsed().as_secs_f32() / DIALOG_OPEN_DURATION).min(1.0);
                    (ease_out_back(t), false)
                }
            } else {
                (1.0, false)
            };

            let overlay_alpha = (progress * 180.0) as u8;
            let scale = 0.9 + 0.1 * progress;
            let offset_y = (1.0 - progress) * 20.0;

            // Overlay backdrop (rendered below the dialog window)
            egui::Area::new(egui::Id::new(format!("overlay_{}", dialog_id)))
                .fixed_pos(screen_rect.min)
                .order(egui::Order::Middle)
                .interactable(true)
                .show(ctx, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
                    let overlay_color = self.palette().bg_overlay;
                    let bg = Color32::from_rgba_premultiplied(
                        overlay_color.r(),
                        overlay_color.g(),
                        overlay_color.b(),
                        overlay_alpha,
                    );
                    ui.painter().rect_filled(rect, 0.0, bg);
                    if response.clicked() && !is_closing {
                        self.apply_effect(serde_json::json!({
                            "effect": "view.close",
                            "target": dialog_id,
                            "payload": {}
                        }));
                    }
                });

            let dialog_width = (screen_rect.width() * 0.45).clamp(380.0, 560.0);
            let content_alpha = (progress * 255.0) as u8;

            egui::Window::new(format!("  "))
                .id(egui::Id::new(format!("dialog_{}", dialog_id)))
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, offset_y])
                .fixed_size([dialog_width * scale, 0.0])
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(
                            self.cfg.dialog_bg.r(),
                            self.cfg.dialog_bg.g(),
                            self.cfg.dialog_bg.b(),
                            content_alpha,
                        ))
                        .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(
                            self.c_border().r(), self.c_border().g(), self.c_border().b(), content_alpha,
                        )))
                        .corner_radius(CornerRadius::same(self.cfg.dialog_corner_radius as u8))
                        .inner_margin(Margin::same(self.cfg.dialog_margin as i8))
                        .shadow(egui::epaint::Shadow {
                            offset: [0, (8.0 * progress) as i8].into(),
                            blur: (24.0 * progress) as u8,
                            spread: (4.0 * progress) as u8,
                            color: Color32::from_rgba_premultiplied(0, 0, 0, (25.0 * progress) as u8),
                        }),
                )
                .show(ctx, |ui| {
                    ui.set_opacity(progress);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&dialog_label)
                                .font(FontId::proportional(18.0))
                                .color(self.c_text_primary())
                                .strong(),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                let close_btn_text_sec = self.c_text_secondary();
                                let close_btn_error = self.c_error_text();
                                if animated_close_button(ui, &mut self.close_button_hover_t, close_btn_text_sec, close_btn_error).clicked() && !is_closing {
                                    self.apply_effect(serde_json::json!({
                                        "effect": "view.close",
                                        "target": dialog_id,
                                        "payload": {}
                                    }));
                                }
                            },
                        );
                    });
                    ui.add_space(20.0);

                    let children = dialog.children.clone();
                    self.render_children_with_layout(ui, dialog, &children, "column", 12.0);
                });

            if progress < 1.0 || is_closing {
                ctx.request_repaint();
            }
        }
    }

    // ─── Toast notifications ─────────────────────────────────────────────────

    fn render_toast(&mut self, ctx: &egui::Context) {
        let should_dismiss = self
            .toast
            .as_ref()
            .map(|t| t.created_at.elapsed().as_secs_f32() > TOAST_DURATION_SECS)
            .unwrap_or(false);

        if should_dismiss {
            self.toast = None;
            return;
        }

        if let Some(toast) = &self.toast {
            let elapsed = toast.created_at.elapsed().as_secs_f32();

            // Entry: elastic spring (0 -> 0.4s)
            // Exit: smooth fade out with slide down (last 0.6s)
            let (opacity, offset_y) = if elapsed < 0.4 {
                let t = elapsed / 0.4;
                let spring = ease_out_elastic(t);
                (spring.clamp(0.0, 1.0), (1.0 - spring) * 40.0)
            } else if elapsed > TOAST_DURATION_SECS - 0.6 {
                let t = ((elapsed - (TOAST_DURATION_SECS - 0.6)) / 0.6).clamp(0.0, 1.0);
                let ease = ease_out_quart(t);
                (1.0 - ease, ease * 20.0)
            } else {
                (1.0, 0.0)
            };

            let toast_bg_color = self.palette().toast_bg;
            let toast_success_color = self.palette().toast_success;
            let alpha = (opacity * 240.0) as u8;
            let bg = Color32::from_rgba_unmultiplied(
                toast_bg_color.r(),
                toast_bg_color.g(),
                toast_bg_color.b(),
                alpha,
            );
            let text_alpha = (opacity * 255.0) as u8;

            egui::Area::new(egui::Id::new("toast_notification"))
                .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -28.0 + offset_y])
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    let shadow_alpha = (opacity * 20.0) as u8;
                    let frame = egui::Frame::new()
                        .fill(bg)
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin { left: 20, right: 20, top: 12, bottom: 12 })
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 4].into(),
                            blur: 12,
                            spread: 0,
                            color: Color32::from_rgba_premultiplied(0, 0, 0, shadow_alpha),
                        });
                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("\u{e06c}")
                                    .font(FontId::new(14.0, icon_family()))
                                    .color(Color32::from_rgba_unmultiplied(
                                        toast_success_color.r(),
                                        toast_success_color.g(),
                                        toast_success_color.b(),
                                        text_alpha,
                                    )),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(&toast.message)
                                    .font(FontId::proportional(14.0))
                                    .color(Color32::from_rgba_unmultiplied(
                                        255,
                                        255,
                                        255,
                                        text_alpha,
                                    )),
                            );
                        });
                    });
                });

            ctx.request_repaint();
        }
    }

    #[allow(dead_code)]
    fn check_shortcuts(&mut self, ui: &mut egui::Ui) {
        let nodes = self.manifest.nodes.clone();
        let mut triggered: Vec<(SemanticNode, SemanticAction)> = Vec::new();
        for node in &nodes {
            collect_shortcut_triggers(ui, node, &mut triggered);
        }
        for (node, action) in triggered {
            self.invoke_action(&node, &action);
        }
    }
}

impl eframe::App for SemanticApplication {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        while let Ok(effect) = self.remote_effects.try_recv() {
            self.apply_effect(effect);
        }

        // Dispatch WebSocket messages as Lua handler invocations.
        let mut ws_effects: Vec<Value> = Vec::new();
        while let Ok(msg) = self.ws_effect_receiver.try_recv() {
            match msg {
                WsMsg::Connected { conn_id, tx } => {
                    self.ws_senders.insert(conn_id, tx);
                }
                WsMsg::Message { on_message, data, .. } => {
                    if !on_message.is_empty() {
                        let invoke_ctx = InvokeContext::new(&self.state, &self.storage);
                        match self.runtime.invoke(
                            &on_message,
                            &serde_json::Map::from_iter([("data".into(), json!(data))]),
                            &invoke_ctx,
                        ) {
                            Ok(effects) => ws_effects.extend(effects),
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                }
                WsMsg::Closed { on_close, conn_id } => {
                    self.ws_senders.remove(&conn_id);
                    if !on_close.is_empty() {
                        let invoke_ctx = InvokeContext::new(&self.state, &self.storage);
                        match self.runtime.invoke(
                            &on_close,
                            &serde_json::Map::new(),
                            &invoke_ctx,
                        ) {
                            Ok(effects) => ws_effects.extend(effects),
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                }
            }
        }
        for effect in ws_effects {
            self.apply_effect(effect);
        }

        // Dispatch completed async HTTP fetches as Lua callback invocations.
        let mut fetch_effects: Vec<Value> = Vec::new();
        while let Ok(msg) = self.fetch_receiver.try_recv() {
            if !msg.callback.is_empty() {
                let invoke_ctx = InvokeContext::new(&self.state, &self.storage);
                match self.runtime.invoke(
                    &msg.callback,
                    &serde_json::Map::from_iter([("response".into(), msg.response)]),
                    &invoke_ctx,
                ) {
                    Ok(effects) => fetch_effects.extend(effects),
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            ctx.request_repaint();
        }
        for effect in fetch_effects {
            self.apply_effect(effect);
        }

        // Flush deferred viewport commands (window.minimize / window.close).
        let cmds: Vec<egui::ViewportCommand> = std::mem::take(&mut self.deferred_viewport_cmds);
        for cmd in cmds {
            ctx.send_viewport_cmd(cmd);
        }

        // TODO: implement keyboard shortcuts
        // self.check_shortcuts(ui);

        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title.clone()));

        // ─── Global styling ──────────────────────────────────────────────────
        let pal = self.palette().clone();
        let mut visuals = match self.cfg.theme {
            Theme::Dark => egui::Visuals::dark(),
            _ => egui::Visuals::light(),
        };
        visuals.panel_fill = if matches!(self.splash_phase, SplashPhase::Done) {
            pal.bg_primary
        } else {
            SPLASH_BG
        };
        visuals.window_fill = pal.bg_elevated;
        visuals.override_text_color = Some(pal.text_primary);
        visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
        visuals.widgets.inactive.bg_fill = self.cfg.input_bg;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.cfg.input_border);
        visuals.widgets.inactive.corner_radius = CornerRadius::same(self.cfg.input_corner_radius as u8);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.border);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(self.cfg.input_corner_radius as u8);
        visuals.widgets.active.bg_fill = pal.accent_subtle;
        visuals.widgets.active.bg_stroke = Stroke::new(2.0, pal.accent);
        visuals.widgets.active.corner_radius = CornerRadius::same(self.cfg.input_corner_radius as u8);
        visuals.widgets.open.bg_fill = self.cfg.input_bg;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, pal.accent);
        visuals.widgets.open.corner_radius = CornerRadius::same(self.cfg.input_corner_radius as u8);
        visuals.window_shadow = egui::epaint::Shadow::NONE;
        // A plain, consistent light gray for the scroll bar handle, independent of
        // the input/button bg_fill (previously the handle picked up `bg_fill`, which
        // is white for inputs, and the lane background picked up `extreme_bg_color`,
        // which is also what TextEdit uses for its own background -- so tweaking one
        // to fix the scroll bar broke the other). fg_stroke isn't used elsewhere in
        // this hand-painted UI, so it's safe to dedicate to the scroll bar handle.
        let scrollbar_gray = Stroke::new(1.0, Color32::from_rgb(0xC7, 0xCB, 0xD4));
        visuals.widgets.inactive.fg_stroke = scrollbar_gray;
        visuals.widgets.hovered.fg_stroke = scrollbar_gray;
        visuals.widgets.active.fg_stroke = scrollbar_gray;
        ctx.set_visuals(visuals);

        ctx.global_style_mut(|style| {
            // Floating (rather than solid) so the opacity fields below are honored --
            // `solid()` always paints its handle/background at full opacity with no
            // way to fade it, which is why it never disappeared.
            let scroll = &mut style.spacing.scroll;
            *scroll = egui::style::ScrollStyle::floating();
            scroll.bar_width = 6.0;
            scroll.floating_width = 6.0;
            // Reserve a small lane so the bar sits in its own space beside the
            // cards instead of floating on top of them.
            scroll.floating_allocated_width = 10.0;
            // Handle color comes from fg_stroke (a plain gray) instead of bg_fill.
            scroll.foreground_color = true;
            // No background/lane fill at all, ever.
            scroll.dormant_background_opacity = 0.0;
            scroll.active_background_opacity = 0.0;
            scroll.interact_background_opacity = 0.0;
            // Fully invisible until the pointer is over the scroll area, then a
            // faint hint, going fully opaque only while actually dragging/hovering
            // the bar itself.
            scroll.dormant_handle_opacity = 0.0;
            scroll.active_handle_opacity = 0.4;
            scroll.interact_handle_opacity = 1.0;
        });


        // ─── Titlebar ────────────────────────────────────────────────────────
        self.render_titlebar(ui);

        let screens = self.screens().into_iter().cloned().collect::<Vec<_>>();
        let has_sidebar = screens.len() > 1;

        // ─── Sidebar ─────────────────────────────────────────────────────────
        if has_sidebar {
            egui::Panel::left("navigation")
                .exact_size(self.cfg.sidebar_width)
                .frame(
                    egui::Frame::new()
                        .fill(self.cfg.sidebar_bg)
                        .inner_margin(Margin { left: 12, right: 12, top: 16, bottom: 16 })
                        .stroke(Stroke::new(1.0, pal.border)),
                )
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    let item_width = ui.available_width();
                    let dt = ui.input(|i| i.stable_dt);

                    for (idx, screen) in screens.iter().enumerate() {
                        let is_active = idx == self.active_screen;
                        let screen_icon = screen.icon.as_deref().and_then(icon_char);
                        let label = screen.label.clone();

                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(item_width, 36.0),
                            egui::Sense::click(),
                        );

                        if response.hovered() && !is_active {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }

                        let anim = self.sidebar_anims.entry(idx).or_default();
                        let hover_target = if response.hovered() && !is_active { 1.0 } else { 0.0 };
                        let active_target = if is_active { 1.0 } else { 0.0 };
                        let hover_speed = if hover_target > anim.hover_t { 12.0 } else { 8.0 };
                        let active_speed = 8.0;

                        if (anim.hover_t - hover_target).abs() > 0.001 {
                            anim.hover_t += (hover_target - anim.hover_t) * (hover_speed * dt).min(1.0);
                            ui.ctx().request_repaint();
                        } else {
                            anim.hover_t = hover_target;
                        }
                        if (anim.active_t - active_target).abs() > 0.001 {
                            anim.active_t += (active_target - anim.active_t) * (active_speed * dt).min(1.0);
                            ui.ctx().request_repaint();
                        } else {
                            anim.active_t = active_target;
                        }
                        let hover_t = anim.hover_t;
                        let active_t = anim.active_t;

                        if ui.is_rect_visible(rect) {
                            let painter = ui.painter();

                            let hover_bg = lerp_color(self.cfg.sidebar_bg, pal.border, 0.5);
                            let fill = if active_t > 0.001 {
                                lerp_color(
                                    lerp_color(self.cfg.sidebar_bg, hover_bg, hover_t),
                                    self.cfg.sidebar_item_active_bg,
                                    active_t,
                                )
                            } else if hover_t > 0.001 {
                                lerp_color(self.cfg.sidebar_bg, hover_bg, hover_t)
                            } else {
                                self.cfg.sidebar_bg
                            };

                            let stroke_alpha = ((hover_t.max(active_t)) * 255.0) as u8;
                            let stroke_color = Color32::from_rgba_unmultiplied(
                                pal.border.r(), pal.border.g(), pal.border.b(), stroke_alpha,
                            );
                            let stroke = Stroke::new(1.0, stroke_color);

                            painter.rect(rect, CornerRadius::same(8), fill, stroke, egui::StrokeKind::Inside);

                            let text_color = lerp_color(self.cfg.sidebar_text, self.cfg.sidebar_text_active, (hover_t + active_t).min(1.0));

                            let mut x = rect.left() + 12.0;
                            if let Some(ic) = screen_icon {
                                painter.text(
                                    egui::pos2(x + 7.0, rect.center().y),
                                    egui::Align2::CENTER_CENTER,
                                    ic.to_string(),
                                    FontId::new(14.0, icon_family()),
                                    text_color,
                                );
                                x += 24.0;
                            }
                            painter.text(
                                egui::pos2(x, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                &label,
                                FontId::proportional(14.0),
                                text_color,
                            );
                        }

                        if !is_active && response.clicked() {
                            self.screen_transition = Some(ScreenTransition {
                                started: Instant::now(),
                            });
                            self.active_screen = idx;
                        }

                        ui.add_space(2.0);
                    }
                });
        }

        // ─── Screen transition ────────────────────────────────────────────────
        let screen_opacity = if let Some(ref transition) = self.screen_transition {
            let elapsed = transition.started.elapsed().as_secs_f32();
            if elapsed >= SCREEN_TRANSITION_DURATION {
                self.screen_transition = None;
                1.0
            } else {
                let t = elapsed / SCREEN_TRANSITION_DURATION;
                ui.ctx().request_repaint();
                ease_out_cubic(t)
            }
        } else {
            1.0
        };

        let screen_offset_y = (1.0 - screen_opacity) * 12.0;

        // ─── Main content ────────────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.bg_primary)
                    .inner_margin(Margin {
                        left: self.cfg.content_margin_x as i8,
                        right: self.cfg.content_margin_x as i8,
                        top: self.cfg.content_margin_y as i8,
                        bottom: self.cfg.content_margin_y as i8,
                    }),
            )
            .show(ui, |ui| {
                ui.set_opacity(screen_opacity);
                ui.add_space(screen_offset_y);

                let active_idx = self.active_screen;
                let screen = if active_idx < screens.len() {
                    Some(screens[active_idx].clone())
                } else {
                    screens.first().cloned()
                };

                if let Some(screen) = screen {
                    ui.label(
                        egui::RichText::new(&screen.label)
                            .font(FontId::proportional(self.cfg.font_size_title))
                            .color(self.c_text_primary())
                            .strong(),
                    );
                    ui.add_space(24.0);
                    ui.separator();
                    ui.add_space(24.0);

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.render_screen_content(ui, &screen);

                        if let Some(error) = &self.error.clone() {
                            ui.add_space(16.0);
                            let error_frame = egui::Frame::new()
                                .fill(self.c_error_bg())
                                .stroke(Stroke::new(1.0, self.c_error_border()))
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(Margin::same(14));
                            error_frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("!")
                                            .font(FontId::proportional(14.0))
                                            .color(self.c_error_text())
                                            .strong(),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        egui::RichText::new(error)
                                            .font(FontId::proportional(13.0))
                                            .color(self.c_error_text()),
                                    );
                                });
                            });
                        }
                    });
                }
            });

        // ─── Modal dialogs (over everything) ─────────────────────────────────
        self.render_dialogs(&ctx);

        // ─── Modal MCP setup (above application dialogs) ─────────────────────
        self.render_mcp_modal(&ctx);

        // ─── Toast (topmost) ─────────────────────────────────────────────────
        self.render_toast(&ctx);

        // ─── Splash screen (absolutely topmost) ──────────────────────────────
        self.render_splash(&ctx);
    }
}

// ─── Reusable UI components ─────────────────────────────────────────────────

fn animated_primary_button(
    ui: &mut egui::Ui,
    label: &str,
    icon: Option<char>,
    anim: &ButtonAnim,
    accent: Color32,
    accent_hover: Color32,
    text_on_accent: Color32,
    corner_radius: u8,
) -> egui::Response {
    let text_width = label.len() as f32 * 8.0 + 40.0;
    let icon_extra = if icon.is_some() { 24.0 } else { 0.0 };
    let desired_size = Vec2::new(text_width + icon_extra, 38.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let rounding = CornerRadius::same(corner_radius);

        let hover_id = response.id.with("hover");
        let hover_t = ui.ctx().animate_bool_with_time(hover_id, response.hovered(), 0.15);

        let press_id = response.id.with("press");
        let press_t = ui.ctx().animate_bool_with_time(press_id, response.is_pointer_button_down_on(), 0.08);

        let base_color = lerp_color(accent, accent_hover, hover_t);
        let pressed_color = lerp_color(accent_hover, Color32::from_rgb(0x1E, 0x45, 0xC0), 0.5);
        let bg = lerp_color(base_color, pressed_color, press_t);

        let shadow_alpha = (hover_t * 35.0) as u8;
        if shadow_alpha > 0 {
            painter.rect_filled(
                rect.translate(Vec2::new(0.0, 2.0)),
                CornerRadius::same(corner_radius + 1),
                Color32::from_rgba_premultiplied(accent.r(), accent.g(), accent.b(), shadow_alpha),
            );
        }

        painter.rect_filled(rect, rounding, bg);

        if let Some(icon_ch) = icon {
            let icon_x = rect.left() + 16.0 + 7.0;
            painter.text(
                egui::pos2(icon_x, rect.center().y),
                egui::Align2::CENTER_CENTER,
                icon_ch.to_string(),
                FontId::new(15.0, icon_family()),
                text_on_accent,
            );
            let text_center = egui::pos2(
                rect.left() + icon_extra + (rect.width() - icon_extra) / 2.0,
                rect.center().y,
            );
            painter.text(
                text_center,
                egui::Align2::CENTER_CENTER,
                label,
                FontId::proportional(14.0),
                text_on_accent,
            );
        } else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                FontId::proportional(14.0),
                text_on_accent,
            );
        }
    }

    let _ = anim;
    response
}

fn animated_close_button(
    ui: &mut egui::Ui,
    hover_t: &mut f32,
    text_secondary: Color32,
    error_text: Color32,
) -> egui::Response {
    let size = Vec2::splat(28.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let dt = ui.input(|i| i.stable_dt);
    let target = if response.hovered() { 1.0 } else { 0.0 };
    let speed = if target > *hover_t { 12.0 } else { 8.0 };
    if (*hover_t - target).abs() > 0.001 {
        *hover_t += (target - *hover_t) * (speed * dt).min(1.0);
        ui.ctx().request_repaint();
    } else {
        *hover_t = target;
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bg_alpha = (*hover_t * 255.0) as u8;
        let bg = Color32::from_rgba_unmultiplied(0xF3, 0xF4, 0xF6, bg_alpha);

        painter.rect_filled(rect, CornerRadius::same(6), bg);

        let icon_color = lerp_color(text_secondary, error_text, *hover_t * 0.6);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "\u{e1b2}",
            FontId::new(14.0, icon_family()),
            icon_color,
        );
    }
    response
}

// ─── Utilities ──────────────────────────────────────────────────────────────

fn form_key(node_id: &str, action_id: &str, parameter_name: &str) -> String {
    format!("{node_id}.{action_id}.{parameter_name}")
}

include!(concat!(env!("OUT_DIR"), "/icon_map.rs"));

/// Render a read-only code snippet with a tinted background.
fn mcp_section_header(ui: &mut egui::Ui, pal: &config::ColorPalette, icon: char, title: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(icon.to_string())
                .font(FontId::new(14.0, icon_family()))
                .color(pal.text_secondary),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(title)
                .font(FontId::proportional(13.0))
                .color(pal.text_primary)
                .strong(),
        );
    });
}

fn code_block(ui: &mut egui::Ui, pal: &config::ColorPalette, mono: &FontId, text: &str) {
    let icon_copy = icon_char("copy").unwrap_or('⎘');
    let icon_check = icon_char("check").unwrap_or('✓');
    let copy_id = egui::Id::new(("code_block_copied", text));
    let copied_at: Option<Instant> = ui.ctx().data(|d| d.get_temp(copy_id));
    let just_copied = copied_at
        .map(|t| t.elapsed().as_secs_f32() < 1.5)
        .unwrap_or(false);

    egui::Frame::new()
        .fill(pal.bg_primary)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin { left: 14, right: 10, top: 10, bottom: 10 })
        .show(ui, |ui| {
            let full_width = ui.available_width();
            ui.horizontal(|ui| {
                // Code lines stacked vertically, left side
                let text_width = full_width - 28.0;
                ui.vertical(|ui| {
                    ui.set_max_width(text_width);
                    for line in text.lines() {
                        ui.label(
                            egui::RichText::new(line)
                                .font(mono.clone())
                                .color(pal.text_primary),
                        );
                    }
                });

                // Copy icon, right side
                let (icon_ch, icon_color) = if just_copied {
                    (icon_check, pal.accent)
                } else {
                    (icon_copy, pal.text_secondary)
                };
                let btn = ui.add(
                    egui::Label::new(
                        egui::RichText::new(icon_ch.to_string())
                            .font(FontId::new(15.0, icon_family()))
                            .color(icon_color),
                    )
                    .sense(egui::Sense::click()),
                );
                if btn.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if btn.on_hover_text("Copier").clicked() {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(text);
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(copy_id, Instant::now()));
                }
                if just_copied {
                    ui.ctx().request_repaint();
                }
            });
        });
}

fn icon_char(name: &str) -> Option<char> {
    ICON_MAP
        .binary_search_by_key(&name, |(k, _)| k)
        .ok()
        .map(|i| ICON_MAP[i].1)
}

#[allow(dead_code)]
fn collect_shortcut_triggers(
    ui: &egui::Ui,
    node: &SemanticNode,
    out: &mut Vec<(SemanticNode, SemanticAction)>,
) {
    for action in &node.actions {
        if let Some(shortcut) = &action.shortcut {
            if shortcut_pressed(ui, shortcut) {
                out.push((node.clone(), action.clone()));
            }
        }
    }
    for child in &node.children {
        collect_shortcut_triggers(ui, child, out);
    }
}

#[allow(dead_code)]
fn shortcut_pressed(ui: &egui::Ui, shortcut: &str) -> bool {
    let parts: Vec<&str> = shortcut.split('+').collect();
    let (modifiers, key_str) = match parts.split_last() {
        Some((k, mods)) => (*k, mods),
        None => return false,
    };

    let key = match modifiers.to_ascii_lowercase().as_str() {
        "return" | "enter" => egui::Key::Enter,
        "escape" | "esc" => egui::Key::Escape,
        "tab" => egui::Key::Tab,
        "space" => egui::Key::Space,
        "delete" | "del" => egui::Key::Delete,
        "backspace" => egui::Key::Backspace,
        s if s.len() == 1 => {
            let c = s.chars().next().unwrap();
            match c {
                'a' => egui::Key::A, 'b' => egui::Key::B, 'c' => egui::Key::C,
                'd' => egui::Key::D, 'e' => egui::Key::E, 'f' => egui::Key::F,
                'g' => egui::Key::G, 'h' => egui::Key::H, 'i' => egui::Key::I,
                'j' => egui::Key::J, 'k' => egui::Key::K, 'l' => egui::Key::L,
                'm' => egui::Key::M, 'n' => egui::Key::N, 'o' => egui::Key::O,
                'p' => egui::Key::P, 'q' => egui::Key::Q, 'r' => egui::Key::R,
                's' => egui::Key::S, 't' => egui::Key::T, 'u' => egui::Key::U,
                'v' => egui::Key::V, 'w' => egui::Key::W, 'x' => egui::Key::X,
                'y' => egui::Key::Y, 'z' => egui::Key::Z,
                _ => return false,
            }
        }
        _ => return false,
    };

    let mut want_cmd = false;
    let mut want_ctrl = false;
    let mut want_shift = false;
    let mut want_alt = false;
    for m in key_str {
        match m.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" => want_cmd = true,
            "ctrl" | "control" => want_ctrl = true,
            "shift" => want_shift = true,
            "alt" | "option" => want_alt = true,
            _ => {}
        }
    }

    ui.input(|i| {
        let m = &i.modifiers;
        let mod_match = m.command == want_cmd
            && m.ctrl == want_ctrl
            && m.shift == want_shift
            && m.alt == want_alt;
        mod_match && i.key_pressed(key)
    })
}

fn coerce_value(text: &str, parameter: &ActionParameter) -> Result<Value, String> {
    match parameter.kind.as_str() {
        "string" => Ok(Value::String(text.into())),
        "number" => text
            .parse::<f64>()
            .map(|n| serde_json::json!(n))
            .map_err(|_| format!("\"{}\" must be a number", parameter.name)),
        "boolean" => text
            .parse::<bool>()
            .map(Value::Bool)
            .map_err(|_| format!("\"{}\" must be true or false", parameter.name)),
        _ => Err(format!(
            "Type `{}` is not yet supported",
            parameter.kind
        )),
    }
}
