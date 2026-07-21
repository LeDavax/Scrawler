# Scrawler

Scrawler is a portable runtime for applications that want to be genuinely
agent-native. Instead of exposing screenshots, DOM selectors, or a bespoke API
for every agent, an application declares a semantic tree and typed actions.

This repository is being built as a hackathon prototype. It reads and validates
a semantic XML manifest, then exposes its semantic tree through a local MCP
server. Declared actions run in a restricted local Lua runtime and return a
portable structured effect.

## Current milestone

```text
app.xml + actions.lua -> Scrawler parser -> semantic tree -> native UI or MCP -> structured effect
```

Run the included reference manifest:

```bash
cargo run -- inspect examples/mail/app.xml
```

The command prints the semantic tree that a future MCP client will discover.
No API key, native executable, or operating-system-specific integration is
required for this step.

## Native reference application

Scrawler can render the declared application as a native Rust window:

```bash
cargo run -- run examples/mail/app.xml
```

The XML describes the screen and its buttons. The renderer creates parameter
fields from the typed action parameters; when the user clicks a button, the
same manifest validation and Lua handler contract is used. The returned effect
is applied by the native application (the mail example opens its compose
window). This is the user-facing application, not an MCP management dashboard.

## Local MCP server

Start the stdio MCP server with the same manifest:

```bash
cargo run -- serve examples/mail/app.xml
```

It exposes two tools:

- `scrawler_get_semantic_tree` returns the complete tree.
- `scrawler_invoke_action` validates that a node, action, and its typed
  arguments were declared in the manifest, then calls the declared Lua handler.

The process writes only JSON-RPC protocol messages to standard output. This is
important when configuring it in an MCP host.

### Making MCP actions visible in the native application

Start the native app first:

```bash
cargo run -- run examples/mail/app.xml
```

Then let Claude, VS Code, or another MCP host start `scrawler serve ...` as
usual. After a validated Lua action, the MCP bridge forwards its structured
effect over a loopback-only local channel to the open window. The tool result
reports `visual_sync: "delivered to the open native application"` when the
effect was rendered.

The mail reference app currently declares these agent and user capabilities:

- `compose_message(recipient)`
- `set_body(body)`
- `send_message()`

## Manifest model

`app.xml` declares components and their allowed capabilities and references a
nearby `actions.lua` file with `actions="actions.lua"`. `actions.lua` holds the
implementation for declared handlers. Scrawler checks the manifest before
dispatching a Lua function, so an agent never receives arbitrary handler access.

Lua returns an effect rather than manipulating a platform directly:

```json
{
  "effect": "ui.open",
  "target": "compose-window",
  "payload": { "recipient": "alice@example.com" }
}
```

The initial runtime provides only `context.ui.open`. Lua's `io`, `os`,
`package`, and `debug` libraries are not loaded.

## Roadmap

1. Parse and validate XML manifests. **Current**
2. Add a local MCP server that exposes the semantic tree. **Current**
3. Add a restricted Lua runtime for declared actions. **Current**
4. Add a native reference UI rendered from the semantic tree. **Current**
5. Add a local IPC bridge so an external MCP client controls the already-open
   native window and shares its visible state.
