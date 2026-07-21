use crate::ipc;
use crate::manifest::{AppManifest, SemanticAction, SemanticNode, find_node_mut};
use crate::runtime::{InvokeContext, LuaRuntime};
use crate::storage::AppStorage;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};

const PROTOCOL_VERSION: &str = "2025-03-26";

pub struct McpServer {
    manifest: AppManifest,
    runtime: LuaRuntime,
    initialized: bool,
    hidden_nodes: HashSet<String>,
    storage: AppStorage,
    state: HashMap<String, String>,
}

impl McpServer {
    pub fn new(manifest: AppManifest, runtime: LuaRuntime) -> Self {
        let storage = AppStorage::new(&manifest.id);
        // Mirror the renderer's initialisation: state defaults declared in
        // `app.xml` must be visible to `context.state.get` from the very first
        // agent-invoked action, exactly like they are in the native window.
        let mut state = HashMap::new();
        for entry in &manifest.state {
            state.insert(entry.id.clone(), entry.default.clone());
        }
        Self {
            manifest,
            runtime,
            initialized: false,
            hidden_nodes: HashSet::new(),
            storage,
            state,
        }
    }

    /// Applies `manifest.*` and `state.*` effects locally so that
    /// `scrawler_get_semantic_tree` reflects mutations and subsequent
    /// `scrawler_invoke_action` calls see up-to-date state, even when no
    /// native window is running to apply these effects itself.
    fn apply_manifest_effects(&mut self, effects: &[Value]) {
        for effect in effects {
            match (
                effect.get("effect").and_then(Value::as_str),
                effect.get("target").and_then(Value::as_str),
            ) {
                (Some("state.set"), Some(key)) => {
                    let value = effect
                        .pointer("/payload/value")
                        .map(crate::runtime::json_to_state_string)
                        .unwrap_or_default();
                    self.state.insert(key.into(), value);
                }
                (Some("view.open"), _) => {
                    if let Some(values) =
                        effect.pointer("/payload/state").and_then(Value::as_object)
                    {
                        for (key, value) in values {
                            self.state
                                .insert(key.clone(), crate::runtime::json_to_state_string(value));
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
                    if let Some(options) = effect.pointer("/payload/options").and_then(Value::as_array) {
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
                _ => {}
            }
        }
    }

    pub fn handle_message(&mut self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);

        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return id
                .map(|request_id| protocol_error(request_id, -32600, "Invalid JSON-RPC version"));
        }

        match method {
            Some("initialize") => {
                self.initialized = true;
                id.map(|request_id| self.initialize(request_id))
            }
            Some("notifications/initialized") => {
                self.initialized = true;
                None
            }
            Some(_) if !self.initialized => id.map(|request_id| {
                protocol_error(
                    request_id,
                    -32002,
                    "Server is not initialized; send initialize first",
                )
            }),
            Some("tools/list") => id.map(|request_id| self.list_tools(request_id)),
            Some("tools/call") => id.map(|request_id| self.call_tool(request_id, &message)),
            Some("ping") => id.map(|request_id| success_response(request_id, json!({}))),
            Some(other) => id.map(|request_id| {
                protocol_error(request_id, -32601, &format!("Method not found: {other}"))
            }),
            None => id.map(|request_id| protocol_error(request_id, -32600, "Missing method")),
        }
    }

    fn initialize(&self, id: Value) -> Value {
        success_response(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "scrawler",
                    "title": "Scrawler Semantic Runtime",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": concat!(
                    "WORKFLOW: call get_tree first, then invoke_action.\n",
                    "get_tree returns nodes (id, role, label) and their actions (id, description, params).\n",
                    "invoke_action(node_id, action_id, arguments?) runs the action and returns structured effects.\n",
                    "Each effect has: effect (type), target (id), payload (data). ",
                    "The UI may be offline; effects are still returned and applied to app state."
                )
            }),
        )
    }

    fn list_tools(&self, id: Value) -> Value {
        success_response(
            id,
            json!({
                "tools": [
                    {
                        "name": "scrawler_get_semantic_tree",
                        "title": "Get Semantic Tree",
                        "description": "Read the app structure: nodes (id, role, label) and their available actions with typed parameters. Always call this before invoke_action.",
                        "inputSchema": {
                            "type": "object",
                            "additionalProperties": false
                        },
                        "outputSchema": {
                            "type": "object",
                            "properties": {
                                "app_id":   { "type": "string" },
                                "app_name": { "type": "string" },
                                "tree":     { "type": "string", "description": "Compact text representation of nodes and actions" }
                            },
                            "additionalProperties": false
                        },
                        "annotations": {
                            "readOnlyHint": true,
                            "destructiveHint": false,
                            "idempotentHint": true,
                            "openWorldHint": false
                        }
                    },
                    {
                        "name": "scrawler_invoke_action",
                        "title": "Invoke Action",
                        "description": "Run an action declared on a node. Returns structured effects (each with effect type, target, and payload). Errors if node_id/action_id is unknown or arguments are invalid.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "node_id": {
                                    "type": "string",
                                    "description": "Node id from get_tree"
                                },
                                "action_id": {
                                    "type": "string",
                                    "description": "Action id from get_tree"
                                },
                                "arguments": {
                                    "type": "object",
                                    "description": "Action parameters (omit or pass {} if none required)",
                                    "additionalProperties": true
                                }
                            },
                            "required": ["node_id", "action_id"],
                            "additionalProperties": false
                        },
                        "outputSchema": {
                            "type": "object",
                            "properties": {
                                "effects": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "effect":  { "type": "string" },
                                            "target":  { "type": "string" },
                                            "payload": {}
                                        },
                                        "required": ["effect", "target"],
                                        "additionalProperties": false
                                    }
                                },
                                "ui_online": { "type": "boolean" }
                            },
                            "additionalProperties": false
                        },
                        "annotations": {
                            "readOnlyHint": false,
                            "destructiveHint": false,
                            "idempotentHint": false,
                            "openWorldHint": false
                        }
                    }
                ]
            }),
        )
    }

    fn call_tool(&mut self, id: Value, message: &Value) -> Value {
        let params = match message.get("params").and_then(Value::as_object) {
            Some(params) => params,
            None => {
                return protocol_error(id, -32602, "tools/call requires an object params field");
            }
        };

        let tool_name = match params.get("name").and_then(Value::as_str) {
            Some(name) => name,
            None => return protocol_error(id, -32602, "tools/call requires a string name"),
        };

        let arguments = params
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        match tool_name {
            "scrawler_get_semantic_tree" => {
                let tree = format_semantic_tree(&self.manifest);
                let structured = json!({
                    "app_id":   self.manifest.id,
                    "app_name": self.manifest.name,
                    "tree":     tree
                });
                tool_success(id, tree, structured)
            }
            "scrawler_invoke_action" => self.invoke_action(id, arguments),
            other => protocol_error(id, -32602, &format!("Unknown tool: {other}")),
        }
    }

    fn invoke_action(&mut self, id: Value, arguments: Map<String, Value>) -> Value {
        let node_id = match arguments.get("node_id").and_then(Value::as_str) {
            Some(v) => v.to_owned(),
            None => return tool_error(id, "`node_id` must be a string"),
        };
        let action_id = match arguments.get("action_id").and_then(Value::as_str) {
            Some(v) => v.to_owned(),
            None => return tool_error(id, "`action_id` must be a string"),
        };
        let action_arguments: Map<String, Value> = arguments
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let (handler, validated) = {
            let node = match find_node(&self.manifest.nodes, &node_id) {
                Some(node) => node,
                None => return tool_error(id, format!("Unknown node: {node_id}")),
            };
            let action = match node.actions.iter().find(|a| a.id == action_id) {
                Some(action) => action,
                None => return tool_error(id, format!("Node `{node_id}` has no action `{action_id}`")),
            };
            if let Err(message) = validate_arguments(action, &action_arguments) {
                return tool_error(id, message);
            }
            (action.handler.clone(), action_arguments)
        };

        let invoke_ctx = InvokeContext::new(&self.state, &self.storage);
        let effects = match self.runtime.invoke(&handler, &validated, &invoke_ctx) {
            Ok(effects) => effects,
            Err(error) => return tool_error(id, error.to_string()),
        };

        self.apply_manifest_effects(&effects);

        let mut all_delivered = true;
        for effect in &effects {
            if ipc::send_effect(effect).is_err() {
                all_delivered = false;
            }
        }

        let mut lines: Vec<String> = effects.iter().map(|e| {
            let kind = e["effect"].as_str().unwrap_or("?");
            let target = e["target"].as_str().unwrap_or("?");
            match e.get("payload").filter(|p| !p.is_null()) {
                Some(p) => format!("{kind} → {target} {}", serde_json::to_string(p).unwrap_or_default()),
                None    => format!("{kind} → {target}"),
            }
        }).collect();
        if !all_delivered {
            lines.push("warning: ui offline — start `scrawler run <app.xml>`".into());
        }
        let text = lines.join("\n");

        let structured = json!({
            "effects": effects,
            "ui_online": all_delivered
        });
        tool_success(id, text, structured)
    }
}

pub fn run_stdio_server(manifest: AppManifest, runtime: LuaRuntime) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut server = McpServer::new(manifest, runtime);
    let mut output = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle_message(message),
            Err(error) => Some(protocol_error(
                Value::Null,
                -32700,
                &format!("Parse error: {error}"),
            )),
        };

        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response)?;
            writeln!(output)?;
            output.flush()?;
        }
    }

    Ok(())
}

/// Starts a minimal MCP HTTP server on `127.0.0.1:port` in a background thread.
/// Returns immediately after binding the socket.
pub fn start_http_server(
    manifest: AppManifest,
    runtime: LuaRuntime,
    port: u16,
) -> io::Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))?;
    // LuaRuntime is not Send (contains Rc); rebuild from source inside the thread.
    let lua_source = runtime.source().to_owned();
    let lua_name = runtime.source_name().to_owned();
    std::thread::spawn(move || {
        let rt = match LuaRuntime::from_source(&lua_source, &lua_name) {
            Ok(rt) => rt,
            Err(_) => return,
        };
        let mut server = McpServer::new(manifest, rt);
        for stream in listener.incoming().flatten() {
            handle_http_connection(stream, &mut server);
        }
    });
    Ok(())
}

fn handle_http_connection(mut stream: TcpStream, server: &mut McpServer) {
    let mut headers = String::new();
    let mut buf = [0u8; 1];
    loop {
        if stream.read(&mut buf).is_err() { return; }
        headers.push(buf[0] as char);
        if headers.ends_with("\r\n\r\n") { break; }
        if headers.len() > 8192 { return; }
    }

    let content_length: usize = headers
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    let first_line = headers.lines().next().unwrap_or("");
    let is_options = first_line.starts_with("OPTIONS");
    let is_post_mcp = first_line.starts_with("POST") &&
        (first_line.contains("/mcp") || first_line.contains(" / "));

    if is_options {
        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Accept, Mcp-Session-Id, MCP-Protocol-Version\r\nAccess-Control-Expose-Headers: Mcp-Session-Id\r\n\r\n");
        return;
    }

    if !is_post_mcp || content_length == 0 {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        return;
    }

    let mut body = vec![0u8; content_length];
    if stream.read_exact(&mut body).is_err() { return; }

    let response_json = match serde_json::from_slice::<Value>(&body) {
        Ok(message) => server.handle_message(message),
        Err(error) => Some(protocol_error(
            Value::Null,
            -32700,
            &format!("Parse error: {error}"),
        )),
    };

    let body_bytes = match response_json {
        Some(resp) => serde_json::to_vec(&resp).unwrap_or_default(),
        None => {
            let _ = stream.write_all(b"HTTP/1.1 202 Accepted\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\n\r\n");
            return;
        }
    };

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        body_bytes.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body_bytes);
}

fn format_semantic_tree(manifest: &AppManifest) -> String {
    let mut output = String::with_capacity(512);
    output.push_str(&format!("app:{} id:{}\n", manifest.name, manifest.id));
    for node in &manifest.nodes {
        format_node(&mut output, node, 0);
    }
    output
}

fn format_node(output: &mut String, node: &SemanticNode, depth: usize) {
    let indent = " ".repeat(depth * 2);
    let label = node.aria_label.as_deref().unwrap_or(&node.label);
    output.push_str(&format!("{indent}[{}] id:{} \"{}\"\n", node.role, node.id, label));
    for action in &node.actions {
        format_action(output, action, depth + 1);
    }
    for child in &node.children {
        format_node(output, child, depth + 1);
    }
}

fn format_action(output: &mut String, action: &SemanticAction, depth: usize) {
    let indent = " ".repeat(depth * 2);
    let shortcut = action.shortcut.as_deref()
        .map(|s| format!(" [{}]", s))
        .unwrap_or_default();
    if action.description.is_empty() {
        output.push_str(&format!("{indent}action:{}{shortcut}\n", action.id));
    } else {
        output.push_str(&format!("{indent}action:{}{shortcut} {}\n", action.id, action.description));
    }
    for param in &action.parameters {
        let req = if param.required { "*" } else { "?" };
        if param.description.is_empty() {
            output.push_str(&format!("{indent}  {req}{} {}\n", param.name, param.kind));
        } else {
            output.push_str(&format!("{indent}  {req}{} {} — {}\n", param.name, param.kind, param.description));
        }
    }
}

fn find_node<'a>(nodes: &'a [SemanticNode], wanted_id: &str) -> Option<&'a SemanticNode> {
    for node in nodes {
        if node.id == wanted_id {
            return Some(node);
        }
        if let Some(found) = find_node(&node.children, wanted_id) {
            return Some(found);
        }
    }
    None
}


fn validate_arguments(
    action: &SemanticAction,
    arguments: &Map<String, Value>,
) -> Result<(), String> {
    for parameter in &action.parameters {
        match arguments.get(&parameter.name) {
            None if parameter.required => {
                return Err(format!("Missing required parameter `{}`", parameter.name));
            }
            None => {}
            Some(value) if value_matches_kind(value, &parameter.kind) => {}
            Some(_) => {
                return Err(format!(
                    "Parameter `{}` must be `{}`",
                    parameter.name, parameter.kind
                ));
            }
        }
    }
    Ok(())
}

fn value_matches_kind(value: &Value, kind: &str) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        _ => false,
    }
}

fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn protocol_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn tool_success(id: Value, text: String, structured: Value) -> Value {
    success_response(
        id,
        json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": structured,
            "isError": false
        }),
    )
}

fn tool_error(id: Value, message: impl AsRef<str>) -> Value {
    let msg = message.as_ref();
    success_response(
        id,
        json!({
            "content": [{ "type": "text", "text": msg }],
            "structuredContent": { "error": msg },
            "isError": true
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest;

    fn test_server() -> McpServer {
        let manifest = parse_manifest(
            r#"
            <app id="mail" name="Mail">
              <screen id="inbox" role="screen" label="Inbox">
                <component id="compose" role="button" label="Compose">
                  <action id="compose_message" handler="compose_message">
                    <param name="recipient" type="string" required="true" />
                  </action>
                </component>
              </screen>
            </app>
            "#,
        )
        .expect("test manifest must parse");
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
        .expect("test Lua script must load");
        McpServer::new(manifest, runtime)
    }

    fn request(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn lists_tools_after_initialization() {
        let mut server = test_server();
        let initialize = server
            .handle_message(request(1, "initialize", json!({})))
            .expect("initialize is a request");
        assert_eq!(initialize["result"]["serverInfo"]["name"], "scrawler");

        server.handle_message(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));

        let result = server
            .handle_message(request(2, "tools/list", json!({})))
            .expect("tools/list is a request");
        assert_eq!(
            result["result"]["tools"][0]["name"],
            "scrawler_get_semantic_tree"
        );
    }

    #[test]
    fn semantic_tree_uses_compact_text_format() {
        let mut server = test_server();
        server.handle_message(request(1, "initialize", json!({})));
        server.handle_message(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));

        let result = server
            .handle_message(request(
                2,
                "tools/call",
                json!({ "name": "scrawler_get_semantic_tree", "arguments": {} }),
            ))
            .expect("tools/call is a request");

        let text = result["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result must be text");
        assert!(text.contains("app:Mail id:mail"));
        assert!(text.contains("id:inbox"));
        assert!(text.contains("action:compose_message"));
        assert!(text.contains("*recipient string"));
    }

    #[test]
    fn executes_a_declared_action() {
        let mut server = test_server();
        server.handle_message(request(1, "initialize", json!({})));
        server.handle_message(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));

        let result = server
            .handle_message(request(
                2,
                "tools/call",
                json!({
                    "name": "scrawler_invoke_action",
                    "arguments": {
                        "node_id": "compose",
                        "action_id": "compose_message",
                        "arguments": { "recipient": "alice@example.com" }
                    }
                }),
            ))
            .expect("tools/call is a request");

        assert_eq!(result["result"]["isError"], json!(false));
        let text = result["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result must be text");
        assert!(text.contains("view.open"));
        assert!(text.contains("composer"));
    }

    #[test]
    fn invoke_action_arguments_are_optional() {
        let manifest = parse_manifest(
            r#"
            <app id="t" name="T">
              <component id="btn" role="button" label="Go">
                <action id="go" handler="go_action" />
              </component>
            </app>
            "#,
        )
        .expect("test manifest must parse");
        let runtime = LuaRuntime::from_source(
            r#"
                function go_action(arguments, context)
                    return context.notification.show("done")
                end
            "#,
            "test.lua",
        )
        .expect("test Lua script must load");
        let mut server = McpServer::new(manifest, runtime);
        server.handle_message(request(1, "initialize", json!({})));
        server.handle_message(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));

        let result = server
            .handle_message(request(
                2,
                "tools/call",
                json!({
                    "name": "scrawler_invoke_action",
                    "arguments": { "node_id": "btn", "action_id": "go" }
                }),
            ))
            .expect("tools/call is a request");

        assert_eq!(result["result"]["isError"], json!(false));
    }
}