//! Intentionally restricted Lua runtime for Scrawler.
//!
//! The XML is the authority: it declares which actions exist and which handler
//! they may call. This module only accepts a handler already validated by the
//! MCP server. Lua is used solely to express local application behaviour and
//! produce a structured effect.

use crate::storage::AppStorage;
use mlua::{Function, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value as LuaValue};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fmt;

/// Context passed to every Lua invocation.
/// Holds what handlers may read without producing a side-effect.
pub struct InvokeContext<'a> {
    pub state: &'a HashMap<String, String>,
    pub storage: &'a AppStorage,
}

impl<'a> InvokeContext<'a> {
    pub fn new(state: &'a HashMap<String, String>, storage: &'a AppStorage) -> Self {
        Self { state, storage }
    }
}

/// Human-readable error from loading or executing a Lua script.
#[derive(Debug)]
pub struct RuntimeError(String);

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RuntimeError {}

/// Lua instance bound to a running application.
///
/// One instance is created once at server startup. Functions declared in
/// `actions.lua` remain loaded, but no OS globals or file access are
/// exposed to the script.
pub struct LuaRuntime {
    lua: Lua,
    source: String,
    source_name: String,
}

impl LuaRuntime {
    /// Loads and validates an application script.
    ///
    /// Only TABLE, STRING, and MATH are enabled. Lua libraries that could read
    /// disk, spawn a process, or load external code (`io`, `os`, `package`,
    /// `debug`) are absent.
    pub fn from_source(source: &str, source_name: &str) -> Result<Self, RuntimeError> {
        let safe_libraries = StdLib::TABLE | StdLib::STRING | StdLib::MATH;
        let lua = Lua::new_with(safe_libraries, LuaOptions::default())
            .map_err(|error| RuntimeError(format!("Could not create Lua runtime: {error}")))?;

        lua.load(source)
            .set_name(source_name)
            .exec()
            .map_err(|error| {
                RuntimeError(format!(
                    "Could not load Lua script `{source_name}`: {error}"
                ))
            })?;

        Ok(Self { lua, source: source.to_owned(), source_name: source_name.to_owned() })
    }

    /// Rebuilds an independent instance from the same source — required to
    /// move the runtime to an OS thread (Lua is not Send/Sync).
    pub fn rebuild(&self) -> Result<Self, RuntimeError> {
        Self::from_source(&self.source, &self.source_name)
    }

    pub fn source(&self) -> &str { &self.source }
    pub fn source_name(&self) -> &str { &self.source_name }

    /// Calls a Lua function declared by the manifest and converts its return
    /// value into a list of JSON effects. A handler may return either a single
    /// effect `{ effect = "...", target = "..." }` or an array of such effects.
    /// Effects are applied in order.
    pub fn invoke(
        &self,
        handler_name: &str,
        arguments: &Map<String, Value>,
        ctx: &InvokeContext<'_>,
    ) -> Result<Vec<Value>, RuntimeError> {
        let handler: Function = self.lua.globals().get(handler_name).map_err(|error| {
            RuntimeError(format!(
                "Lua handler `{handler_name}` is unavailable: {error}"
            ))
        })?;

        let lua_arguments = self
            .lua
            .to_value(arguments)
            .map_err(|error| RuntimeError(format!("Could not pass arguments to Lua: {error}")))?;
        let context = self.context(ctx)?;

        let raw: LuaValue = handler.call((lua_arguments, context)).map_err(|error| {
            RuntimeError(format!("Lua handler `{handler_name}` failed: {error}"))
        })?;
        let value: Value = self.lua.from_value(raw).map_err(|error| {
            RuntimeError(format!(
                "Lua handler `{handler_name}` must return JSON-compatible data: {error}"
            ))
        })?;

        let effects = match value {
            Value::Array(items) => items,
            single => vec![single],
        };

        for effect in &effects {
            validate_effect(effect)?;
        }
        Ok(effects)
    }

    /// Builds the sole capabilities object Lua receives.
    ///
    /// The contract is generic: `context.state.set`, `context.view.open`,
    /// `context.view.close`, and `context.notification.show`. None of these
    /// functions know about Mail, calendar, or any specific application.
    fn context(&self, ctx: &InvokeContext<'_>) -> Result<Table, RuntimeError> {
        let state = ctx.state;
        let storage = ctx.storage.clone();
        let context = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(format!("Could not create Lua context: {error}")))?;

        // --- context.state ---------------------------------------------------
        let state_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;

        let state_set = self
            .lua
            .create_function(|lua, (key, value): (String, LuaValue)| {
                let value: Value = lua.from_value(value)?;
                lua.to_value(
                    &json!({ "effect": "state.set", "target": key, "payload": { "value": value } }),
                )
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        state_table
            .set("set", state_set)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let snapshot: HashMap<String, String> = state.clone();
        let state_get = self
            .lua
            .create_function(move |lua, key: String| {
                match snapshot.get(&key) {
                    Some(value) => lua.to_value(value),
                    None => Ok(LuaValue::Nil),
                }
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        state_table
            .set("get", state_get)
            .map_err(|error| RuntimeError(error.to_string()))?;

        context
            .set("state", state_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.view ----------------------------------------------------
        let view = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        for (name, effect_name) in [("open", "view.open"), ("close", "view.close")] {
            let effect_name = effect_name.to_owned();
            let function = self
                .lua
                .create_function(move |lua, (target, payload): (String, LuaValue)| {
                    let payload: Value = lua.from_value(payload)?;
                    lua.to_value(
                        &json!({ "effect": effect_name, "target": target, "payload": payload }),
                    )
                })
                .map_err(|error| RuntimeError(error.to_string()))?;
            view.set(name, function)
                .map_err(|error| RuntimeError(error.to_string()))?;
        }
        context
            .set("view", view)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.manifest ------------------------------------------------
        let manifest_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;

        let node_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;

        let set_label = self
            .lua
            .create_function(|lua, (id, label): (String, String)| {
                lua.to_value(
                    &json!({ "effect": "manifest.set_label", "target": id, "payload": { "label": label } }),
                )
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        node_table
            .set("set_label", set_label)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let set_visible = self
            .lua
            .create_function(|lua, (id, visible): (String, bool)| {
                lua.to_value(
                    &json!({ "effect": "manifest.set_visible", "target": id, "payload": { "visible": visible } }),
                )
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        node_table
            .set("set_visible", set_visible)
            .map_err(|error| RuntimeError(error.to_string()))?;

        manifest_table
            .set("node", node_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let select_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;

        let set_options = self
            .lua
            .create_function(|lua, (id, options): (String, Vec<String>)| {
                lua.to_value(
                    &json!({ "effect": "manifest.set_options", "target": id, "payload": { "options": options } }),
                )
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        select_table
            .set("set_options", set_options)
            .map_err(|error| RuntimeError(error.to_string()))?;

        manifest_table
            .set("select", select_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        context
            .set("manifest", manifest_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.screen --------------------------------------------------
        let screen_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let navigate = self
            .lua
            .create_function(|lua, id: String| {
                lua.to_value(
                    &json!({ "effect": "screen.navigate", "target": id, "payload": {} }),
                )
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        screen_table
            .set("navigate", navigate)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("screen", screen_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.notification --------------------------------------------
        let notification = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let show = self
            .lua
            .create_function(|lua, message: String| {
                lua.to_value(
                    &json!({ "effect": "notification.show", "target": "notification", "payload": { "message": message } }),
                )
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        notification
            .set("show", show)
            .map_err(|error| RuntimeError(error.to_string()))?;
        let notification_os = self
            .lua
            .create_function(|lua, (title, body): (String, String)| {
                lua.to_value(&json!({
                    "effect": "notification.os",
                    "target": "os",
                    "payload": { "title": title, "body": body }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        notification
            .set("os", notification_os)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("notification", notification)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.browser -------------------------------------------------
        let browser_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let browser_open = self
            .lua
            .create_function(|lua, url: String| {
                lua.to_value(&json!({
                    "effect": "browser.open",
                    "target": url,
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        browser_table
            .set("open", browser_open)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("browser", browser_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.clipboard -----------------------------------------------
        let clipboard_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let clipboard_write = self
            .lua
            .create_function(|lua, text: String| {
                lua.to_value(&json!({
                    "effect": "clipboard.write",
                    "target": "clipboard",
                    "payload": { "text": text }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        clipboard_table
            .set("write", clipboard_write)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("clipboard", clipboard_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.sound ---------------------------------------------------
        let sound_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let sound_play = self
            .lua
            .create_function(|lua, name: String| {
                lua.to_value(&json!({
                    "effect": "sound.play",
                    "target": name,
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        sound_table
            .set("play", sound_play)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("sound", sound_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.window --------------------------------------------------
        let window_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let set_title = self
            .lua
            .create_function(|lua, title: String| {
                lua.to_value(&json!({
                    "effect": "window.set_title",
                    "target": "window",
                    "payload": { "title": title }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        window_table
            .set("set_title", set_title)
            .map_err(|error| RuntimeError(error.to_string()))?;
        let set_badge = self
            .lua
            .create_function(|lua, count: i64| {
                lua.to_value(&json!({
                    "effect": "window.set_badge",
                    "target": "window",
                    "payload": { "count": count }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        window_table
            .set("set_badge", set_badge)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("window", window_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.file ----------------------------------------------------
        let file_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let file_save = self
            .lua
            .create_function(|lua, (filename, content): (String, String)| {
                lua.to_value(&json!({
                    "effect": "file.save",
                    "target": filename,
                    "payload": { "content": content }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        file_table
            .set("save", file_save)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("file", file_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.storage -------------------------------------------------
        let storage_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;

        let s = storage.clone();
        let kv_get = self
            .lua
            .create_function(move |lua, key: String| {
                match s.kv_get(&key) {
                    Some(value) => lua.to_value(&value),
                    None => Ok(LuaValue::Nil),
                }
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        storage_table
            .set("get", kv_get)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let s = storage.clone();
        let kv_get_all = self
            .lua
            .create_function(move |lua, ()| {
                lua.to_value(&Value::Object(s.kv_all()))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        storage_table
            .set("get_all", kv_get_all)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let kv_set = self
            .lua
            .create_function(|lua, (key, value): (String, LuaValue)| {
                let value: Value = lua.from_value(value)?;
                lua.to_value(&json!({
                    "effect": "storage.set",
                    "target": key,
                    "payload": { "value": value }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        storage_table
            .set("set", kv_set)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let kv_delete = self
            .lua
            .create_function(|lua, key: String| {
                lua.to_value(&json!({
                    "effect": "storage.delete",
                    "target": key,
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        storage_table
            .set("delete", kv_delete)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // Files inside the data directory
        let file_sub = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;

        let s = storage.clone();
        let sf_read = self
            .lua
            .create_function(move |lua, path: String| {
                match s.file_read(&path) {
                    Some(content) => lua.to_value(&content),
                    None => Ok(LuaValue::Nil),
                }
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        file_sub
            .set("read", sf_read)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let sf_write = self
            .lua
            .create_function(|lua, (path, content): (String, String)| {
                lua.to_value(&json!({
                    "effect": "storage.file.write",
                    "target": path,
                    "payload": { "content": content }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        file_sub
            .set("write", sf_write)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let sf_delete = self
            .lua
            .create_function(|lua, path: String| {
                lua.to_value(&json!({
                    "effect": "storage.file.delete",
                    "target": path,
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        file_sub
            .set("delete", sf_delete)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let s = storage.clone();
        let sf_list = self
            .lua
            .create_function(move |lua, path: String| {
                let entries = s.dir_list(&path);
                lua.to_value(&entries)
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        file_sub
            .set("list", sf_list)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let sd_create = self
            .lua
            .create_function(|lua, path: String| {
                lua.to_value(&json!({
                    "effect": "storage.dir.create",
                    "target": path,
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        file_sub
            .set("mkdir", sd_create)
            .map_err(|error| RuntimeError(error.to_string()))?;

        storage_table
            .set("file", file_sub)
            .map_err(|error| RuntimeError(error.to_string()))?;

        context
            .set("storage", storage_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.state.toggle -------------------------------------------
        let state_table: Table = context
            .get("state")
            .map_err(|error| RuntimeError(error.to_string()))?;
        let snapshot2: HashMap<String, String> = state.clone();
        let state_toggle = self
            .lua
            .create_function(move |lua, key: String| {
                let current = snapshot2
                    .get(&key)
                    .map(|v| v == "true")
                    .unwrap_or(false);
                let next_value = !current;
                lua.to_value(&json!({
                    "effect": "state.set",
                    "target": key,
                    "payload": { "value": next_value.to_string() }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        state_table
            .set("toggle", state_toggle)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- manifest.node.set_icon -----------------------------------------
        let manifest_table: Table = context
            .get("manifest")
            .map_err(|error| RuntimeError(error.to_string()))?;
        let node_table: Table = manifest_table
            .get("node")
            .map_err(|error| RuntimeError(error.to_string()))?;
        let set_icon = self
            .lua
            .create_function(|lua, (id, icon): (String, String)| {
                lua.to_value(&json!({
                    "effect": "manifest.set_icon",
                    "target": id,
                    "payload": { "icon": icon }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        node_table
            .set("set_icon", set_icon)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.form ---------------------------------------------------
        let form_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let form_reset = self
            .lua
            .create_function(|lua, node_id: String| {
                lua.to_value(&json!({
                    "effect": "form.reset",
                    "target": node_id,
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        form_table
            .set("reset", form_reset)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("form", form_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.clipboard.read -----------------------------------------
        let clipboard_table: Table = context
            .get("clipboard")
            .map_err(|error| RuntimeError(error.to_string()))?;
        let clipboard_read = self
            .lua
            .create_function(|lua, ()| {
                let text = arboard::Clipboard::new()
                    .ok()
                    .and_then(|mut cb| cb.get_text().ok())
                    .unwrap_or_default();
                lua.to_value(&text)
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        clipboard_table
            .set("read", clipboard_read)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.date ---------------------------------------------------
        let date_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let date_now = self
            .lua
            .create_function(|lua, ()| {
                let ts = chrono::Utc::now().timestamp();
                lua.to_value(&ts)
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        date_table
            .set("now", date_now)
            .map_err(|error| RuntimeError(error.to_string()))?;
        let date_format = self
            .lua
            .create_function(|lua, (timestamp, pattern): (i64, String)| {
                use chrono::{DateTime, Utc};
                let dt = DateTime::<Utc>::from_timestamp(timestamp, 0)
                    .unwrap_or_default();
                let formatted = dt.format(&pattern).to_string();
                lua.to_value(&formatted)
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        date_table
            .set("format", date_format)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("date", date_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.json ---------------------------------------------------
        let json_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;
        let json_encode = self
            .lua
            .create_function(|lua, value: LuaValue| {
                let json_value: Value = lua.from_value(value)?;
                let encoded = serde_json::to_string(&json_value)
                    .unwrap_or_else(|_| "null".into());
                lua.to_value(&encoded)
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        json_table
            .set("encode", json_encode)
            .map_err(|error| RuntimeError(error.to_string()))?;
        let json_decode = self
            .lua
            .create_function(|lua, text: String| {
                match serde_json::from_str::<Value>(&text) {
                    Ok(val) => lua.to_value(&val),
                    Err(_) => Ok(LuaValue::Nil),
                }
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        json_table
            .set("decode", json_decode)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("json", json_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.window.minimize / context.window.close -----------------
        let window_table: Table = context
            .get("window")
            .map_err(|error| RuntimeError(error.to_string()))?;
        let win_minimize = self
            .lua
            .create_function(|lua, ()| {
                lua.to_value(&json!({
                    "effect": "window.minimize",
                    "target": "window",
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        window_table
            .set("minimize", win_minimize)
            .map_err(|error| RuntimeError(error.to_string()))?;
        let win_close = self
            .lua
            .create_function(|lua, ()| {
                lua.to_value(&json!({
                    "effect": "window.close",
                    "target": "window",
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        window_table
            .set("close", win_close)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // --- context.http ---------------------------------------------------
        let http_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;

        let fetch_fn = self
            .lua
            .create_function(|lua, (url, opts): (String, LuaValue)| {
                let opts_val: Value = lua.from_value(opts).unwrap_or(json!({}));
                let method = opts_val.pointer("/method").and_then(Value::as_str).unwrap_or("GET").to_uppercase();
                let body = opts_val.pointer("/body").and_then(Value::as_str).map(str::to_owned);
                let timeout_secs = opts_val.pointer("/timeout").and_then(Value::as_u64).unwrap_or(30);

                let mut req = ureq::request(&method, &url);
                req = req.timeout(std::time::Duration::from_secs(timeout_secs));

                // Custom headers
                if let Some(headers) = opts_val.pointer("/headers").and_then(Value::as_object) {
                    for (k, v) in headers {
                        if let Some(v_str) = v.as_str() {
                            req = req.set(k, v_str);
                        }
                    }
                }

                let result: Result<ureq::Response, _> = match body {
                    Some(b) => req.send_string(&b),
                    None => req.call(),
                };

                match result {
                    Ok(resp) => {
                        let status = resp.status();
                        let mut resp_headers: Map<String, Value> = Map::new();
                        for name in resp.headers_names() {
                            if let Some(val) = resp.header(&name) {
                                resp_headers.insert(name, Value::String(val.to_owned()));
                            }
                        }
                        let resp_body = resp.into_string().unwrap_or_default();
                        lua.to_value(&json!({
                            "status": status,
                            "body": resp_body,
                            "headers": resp_headers,
                            "ok": status >= 200 && status < 300
                        }))
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let resp_body = resp.into_string().unwrap_or_default();
                        lua.to_value(&json!({
                            "status": code,
                            "body": resp_body,
                            "headers": {},
                            "ok": false
                        }))
                    }
                    Err(e) => {
                        lua.to_value(&json!({
                            "status": 0,
                            "body": e.to_string(),
                            "headers": {},
                            "ok": false,
                            "error": e.to_string()
                        }))
                    }
                }
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        http_table
            .set("fetch", fetch_fn)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // fetch_async: spawns an OS thread; invokes `callback` with the response on the next frame.
        let fetch_async_fn = self
            .lua
            .create_function(|lua, (url, opts, callback): (String, LuaValue, String)| {
                let opts_val: Value = lua.from_value(opts).unwrap_or(json!({}));
                lua.to_value(&json!({
                    "effect": "http.fetch_async",
                    "target": callback,
                    "payload": {
                        "url": url,
                        "method": opts_val.pointer("/method").and_then(Value::as_str).unwrap_or("GET"),
                        "body": opts_val.pointer("/body").and_then(Value::as_str),
                        "timeout": opts_val.pointer("/timeout").and_then(Value::as_u64).unwrap_or(30),
                        "headers": opts_val.pointer("/headers").cloned().unwrap_or(json!({}))
                    }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        http_table
            .set("fetch_async", fetch_async_fn)
            .map_err(|error| RuntimeError(error.to_string()))?;

        // ws sub-table
        let ws_table = self
            .lua
            .create_table()
            .map_err(|error| RuntimeError(error.to_string()))?;

        let ws_connect = self
            .lua
            .create_function(|lua, (id, url, opts): (String, String, LuaValue)| {
                let opts_val: Value = lua.from_value(opts).unwrap_or(json!({}));
                let on_message = opts_val.pointer("/on_message").and_then(Value::as_str).unwrap_or("").to_owned();
                let on_close = opts_val.pointer("/on_close").and_then(Value::as_str).unwrap_or("").to_owned();
                lua.to_value(&json!({
                    "effect": "http.ws.connect",
                    "target": id,
                    "payload": { "url": url, "on_message": on_message, "on_close": on_close }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        ws_table
            .set("connect", ws_connect)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let ws_send = self
            .lua
            .create_function(|lua, (id, data): (String, String)| {
                lua.to_value(&json!({
                    "effect": "http.ws.send",
                    "target": id,
                    "payload": { "data": data }
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        ws_table
            .set("send", ws_send)
            .map_err(|error| RuntimeError(error.to_string()))?;

        let ws_close = self
            .lua
            .create_function(|lua, id: String| {
                lua.to_value(&json!({
                    "effect": "http.ws.close",
                    "target": id,
                    "payload": {}
                }))
            })
            .map_err(|error| RuntimeError(error.to_string()))?;
        ws_table
            .set("close", ws_close)
            .map_err(|error| RuntimeError(error.to_string()))?;

        http_table
            .set("ws", ws_table)
            .map_err(|error| RuntimeError(error.to_string()))?;
        context
            .set("http", http_table)
            .map_err(|error| RuntimeError(error.to_string()))?;

        Ok(context)
    }
}

/// Validates the minimal contract of an effect before forwarding it to the agent.
/// Prevents Lua from returning an arbitrary string or structure that the
/// renderer would not know how to interpret.
fn validate_effect(effect: &Value) -> Result<(), RuntimeError> {
    let object = effect
        .as_object()
        .ok_or_else(|| RuntimeError("Lua handler must return an effect object".into()))?;
    if !object.get("effect").is_some_and(Value::is_string) {
        return Err(RuntimeError(
            "Lua effect requires a string `effect` field".into(),
        ));
    }
    if !object.get("target").is_some_and(Value::is_string) {
        return Err(RuntimeError(
            "Lua effect requires a string `target` field".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::AppStorage;
    use std::path::PathBuf;

    fn test_storage() -> AppStorage {
        let dir = std::env::temp_dir()
            .join(format!("scrawler_rt_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        AppStorage { dir }
    }

    fn ctx<'a>(state: &'a HashMap<String, String>, storage: &'a AppStorage) -> InvokeContext<'a> {
        InvokeContext::new(state, storage)
    }

    #[test]
    fn state_get_reads_current_value() {
        let runtime = LuaRuntime::from_source(
            r#"
                function check_recipient(arguments, context)
                    local recipient = context.state.get("draft.recipient")
                    if recipient == nil or recipient == "" then
                        return context.notification.show("Recipient is empty")
                    end
                    return context.notification.show("Recipient: " .. recipient)
                end
            "#,
            "test.lua",
        )
        .expect("the test script should load");

        let storage = test_storage();
        let mut state = HashMap::new();
        state.insert("draft.recipient".into(), "alice@example.com".into());

        let effects = runtime
            .invoke("check_recipient", &serde_json::Map::new(), &ctx(&state, &storage))
            .expect("handler should run");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0]["payload"]["message"], "Recipient: alice@example.com");

        let empty_state = HashMap::new();
        let effects = runtime
            .invoke("check_recipient", &serde_json::Map::new(), &ctx(&empty_state, &storage))
            .expect("handler should run");
        assert_eq!(effects[0]["payload"]["message"], "Recipient is empty");
    }

    #[test]
    fn screen_navigate_produces_correct_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function go_settings(arguments, context)
                    return context.screen.navigate("settings")
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("go_settings", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0]["effect"], "screen.navigate");
        assert_eq!(effects[0]["target"], "settings");
    }

    #[test]
    fn manifest_set_label_produces_correct_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function rename_button(arguments, context)
                    return context.manifest.node.set_label("btn", arguments.label)
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke(
                "rename_button",
                &serde_json::Map::from_iter([("label".into(), json!("Send Now"))]),
                &ctx(&HashMap::new(), &test_storage()),
            )
            .expect("handler should run");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0]["effect"], "manifest.set_label");
        assert_eq!(effects[0]["target"], "btn");
        assert_eq!(effects[0]["payload"]["label"], "Send Now");
    }

    #[test]
    fn manifest_set_visible_produces_correct_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function hide_panel(arguments, context)
                    return context.manifest.node.set_visible("panel", false)
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("hide_panel", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0]["effect"], "manifest.set_visible");
        assert_eq!(effects[0]["target"], "panel");
        assert_eq!(effects[0]["payload"]["visible"], false);
    }

    #[test]
    fn manifest_set_options_produces_correct_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function populate_select(arguments, context)
                    return context.manifest.select.set_options("priority", {"Low", "Medium", "High"})
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("populate_select", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0]["effect"], "manifest.set_options");
        assert_eq!(effects[0]["target"], "priority");
        assert_eq!(effects[0]["payload"]["options"][0], "Low");
        assert_eq!(effects[0]["payload"]["options"][2], "High");
    }

    #[test]
    fn calls_state_set_and_returns_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function set_body(arguments, context)
                    return context.state.set("draft.body", arguments.body)
                end
            "#,
            "test.lua",
        )
        .expect("the test script should load");

        let effects = runtime
            .invoke(
                "set_body",
                &serde_json::Map::from_iter([("body".into(), json!("Hello world"))]),
                &ctx(&HashMap::new(), &test_storage()),
            )
            .expect("the declared handler should run");
        assert_eq!(effects.len(), 1);
        let effect = &effects[0];
        assert_eq!(effect["effect"], "state.set");
        assert_eq!(effect["target"], "draft.body");
        assert_eq!(effect["payload"]["value"], "Hello world");
    }

    #[test]
    fn calls_view_open_and_returns_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function compose_message(arguments, context)
                    return context.view.open("composer", {
                        state = { ["draft.recipient"] = arguments.recipient }
                    })
                end
            "#,
            "test.lua",
        )
        .expect("the test script should load");

        let effects = runtime
            .invoke(
                "compose_message",
                &serde_json::Map::from_iter([("recipient".into(), json!("alice@example.com"))]),
                &ctx(&HashMap::new(), &test_storage()),
            )
            .expect("the declared handler should run");
        assert_eq!(effects.len(), 1);
        let effect = &effects[0];
        assert_eq!(effect["effect"], "view.open");
        assert_eq!(effect["target"], "composer");
        assert_eq!(
            effect["payload"]["state"]["draft.recipient"],
            "alice@example.com"
        );
    }

    #[test]
    fn state_toggle_inverts_bool() {
        let runtime = LuaRuntime::from_source(
            r#"
                function toggle_dark(arguments, context)
                    return context.state.toggle("dark_mode")
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let storage = test_storage();
        let mut state = HashMap::new();
        state.insert("dark_mode".into(), "true".into());

        let effects = runtime
            .invoke("toggle_dark", &serde_json::Map::new(), &ctx(&state, &storage))
            .expect("handler should run");
        assert_eq!(effects[0]["effect"], "state.set");
        assert_eq!(effects[0]["target"], "dark_mode");
        assert_eq!(effects[0]["payload"]["value"], "false");

        // Starting from false → true
        state.insert("dark_mode".into(), "false".into());
        let effects = runtime
            .invoke("toggle_dark", &serde_json::Map::new(), &ctx(&state, &storage))
            .expect("handler should run");
        assert_eq!(effects[0]["payload"]["value"], "true");
    }

    #[test]
    fn manifest_set_icon_produces_correct_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function change_icon(arguments, context)
                    return context.manifest.node.set_icon("btn", "star")
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("change_icon", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        assert_eq!(effects[0]["effect"], "manifest.set_icon");
        assert_eq!(effects[0]["target"], "btn");
        assert_eq!(effects[0]["payload"]["icon"], "star");
    }

    #[test]
    fn form_reset_produces_correct_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function reset_form(arguments, context)
                    return context.form.reset("search_bar")
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("reset_form", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        assert_eq!(effects[0]["effect"], "form.reset");
        assert_eq!(effects[0]["target"], "search_bar");
    }

    #[test]
    fn date_now_returns_timestamp() {
        let runtime = LuaRuntime::from_source(
            r#"
                function get_ts(arguments, context)
                    local ts = context.date.now()
                    return context.notification.show("ts=" .. ts)
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("get_ts", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        // Timestamp message should start with "ts=" followed by digits > 0.
        let msg = effects[0]["payload"]["message"].as_str().unwrap_or("");
        assert!(msg.starts_with("ts="), "expected 'ts=...' got '{msg}'");
        let ts: i64 = msg.strip_prefix("ts=").unwrap().parse().unwrap();
        assert!(ts > 1_700_000_000, "timestamp looks wrong: {ts}");
    }

    #[test]
    fn date_format_works() {
        let runtime = LuaRuntime::from_source(
            r#"
                function fmt_date(arguments, context)
                    local formatted = context.date.format(0, "%Y-%m-%d")
                    return context.notification.show(formatted)
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("fmt_date", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        assert_eq!(effects[0]["payload"]["message"], "1970-01-01");
    }

    #[test]
    fn json_encode_decode_roundtrip() {
        let runtime = LuaRuntime::from_source(
            r#"
                function roundtrip(arguments, context)
                    local tbl = { name = "alice", score = 42 }
                    local encoded = context.json.encode(tbl)
                    local decoded = context.json.decode(encoded)
                    return context.notification.show(decoded.name .. "=" .. decoded.score)
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("roundtrip", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        assert_eq!(effects[0]["payload"]["message"], "alice=42");
    }

    #[test]
    fn window_minimize_produces_correct_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function min_win(arguments, context)
                    return context.window.minimize()
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("min_win", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        assert_eq!(effects[0]["effect"], "window.minimize");
    }

    #[test]
    fn http_fetch_get_returns_response() {
        // httpbin.org is unreliable in CI; use a local echo. Instead just verify
        // that the Lua function exists and produces a table with the right shape
        // even when the network call fails (status = 0, ok = false).
        let runtime = LuaRuntime::from_source(
            r#"
                function do_fetch(arguments, context)
                    local resp = context.http.fetch("http://127.0.0.1:0/fail", {})
                    return context.notification.show("ok=" .. tostring(resp.ok))
                end
            "#,
            "test.lua",
        )
        .expect("script should load");

        let effects = runtime
            .invoke("do_fetch", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("handler should run");
        // Network will fail (port 0 / refused), ok must be false.
        assert_eq!(effects[0]["payload"]["message"], "ok=false");
    }

    #[test]
    fn calls_notification_show_and_returns_effect() {
        let runtime = LuaRuntime::from_source(
            r#"
                function send_message(arguments, context)
                    return context.notification.show("Message sent")
                end
            "#,
            "test.lua",
        )
        .expect("the test script should load");

        let effects = runtime
            .invoke("send_message", &serde_json::Map::new(), &ctx(&HashMap::new(), &test_storage()))
            .expect("the declared handler should run");
        assert_eq!(effects.len(), 1);
        let effect = &effects[0];
        assert_eq!(effect["effect"], "notification.show");
        assert_eq!(effect["payload"]["message"], "Message sent");
    }
}
