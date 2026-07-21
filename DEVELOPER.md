# Scrawler Semantic Runtime — Developer Reference

This document is the authoritative technical reference for building applications on the Scrawler Semantic Runtime. It covers the app structure, the Lua runtime, the effect system, the MCP interface, and the persistent storage API.

---

## Architecture overview

A Scrawler Semantic Runtime application is made of three files:

| File | Required | Purpose |
|---|---|---|
| `app.xml` | yes | Declares the UI tree, runtime state, and which actions exist |
| `actions.lua` | yes | Implements the behaviour of each action as Lua functions |
| `manifest.yml` | no | Controls appearance, window dimensions, theme, and build metadata |

At startup, `scrawler run app.xml` parses all three files, validates them, and opens a native window. `scrawler serve app.xml` does the same but instead starts a JSON-RPC MCP server on stdout — no window is opened. Both modes share the same parser and Lua runtime. `manifest.yml` is only read by the renderer and the build tool; the MCP server ignores it entirely.

The renderer and the MCP server are separate OS processes. Each maintains its own in-memory copy of the app tree. Mutations produced by Lua handlers (via effects) are applied independently in each process.

---

## app.xml — UI structure

### Root element

```xml
<app id="com.example.myapp" name="My App" version="0.1" actions="actions.lua">
  ...
</app>
```

| Attribute | Required | Description |
|---|---|---|
| `id` | yes | Stable reverse-DNS identifier. Used as the storage namespace. |
| `name` | yes | Human-readable title shown in the window titlebar. |
| `version` | no | Schema version. Defaults to `"0.1"`. |
| `actions` | yes | Relative path to the Lua file that implements all handlers. |

### State block

Declares the application's runtime state. The renderer initialises its internal dictionary from these defaults at startup.

```xml
<state>
  <value id="draft.recipient" type="string"  default="" />
  <value id="dark_mode"       type="boolean" default="false" />
  <value id="stats.progress"  type="number"  default="0" />
</state>
```

`<value>` attributes:

| Attribute | Required | Values |
|---|---|---|
| `id` | yes | Dot-separated key used in `bind` and `context.state.*` |
| `type` | no | `string` (default), `number`, `boolean` |
| `default` | no | Initial string value. Always stored as a string internally. |

`<state>` must appear before any screen nodes. It cannot be nested.

### Node elements

Nodes are declared with one of five structural XML tags — the semantic meaning comes from the `role` attribute, not the tag name.

```
screen | group | view | component | dialog
```

All five accept the same attributes. The tag choice is cosmetic and only affects readability; the parser treats them identically.

**Common attributes:**

| Attribute | Required | Description |
|---|---|---|
| `id` | yes | Unique identifier within the app. Referenced by actions and effects. |
| `role` | yes | Semantic role. See the [roles table](#roles) below. |
| `label` | yes | Human-readable label rendered on screen and shown to agents. |
| `bind` | no | State key to read/write automatically (e.g. `draft.recipient`). |
| `icon` | no | Lucide icon name (e.g. `pencil`, `send`, `inbox`). |
| `placeholder` | no | Ghost text for `text-input` and `text-area`. |
| `disabled` | no | `"true"` disables user interaction. Default: `"false"`. |
| `readonly` | no | `"true"` makes a field non-editable. Default: `"false"`. |
| `variant` | no | Visual variant: `primary` (default), `secondary`, `destructive`. Used by `button`. |
| `aria-label` | no | Description shown to agents instead of `label`. Use for long explanations. |

### Layout attributes

Container nodes (`screen`, `group`, `view`, `component`, and `dialog`) can describe their
layout instead of relying on the default vertical flow. The same attributes are also useful
on `card` and `list` nodes.

| Attribute | Required | Description |
|---|---|---|
| `layout` | no | `column` (default), `row`, `wrap`, or `grid`. Applies to the node's children. |
| `gap` | no | Space between children, in logical pixels. |
| `padding` | no | Inner padding for `group` and `card`, in logical pixels. |
| `columns` | no | Number of columns when `layout="grid"`. Defaults to `2`. |
| `wrap` | no | `"true"` enables line wrapping for `layout="row"`. |
| `scroll` | no | `"true"` enables vertical scrolling for a `list`. |
| `max-height` | no | Maximum scroll area height for a `list`, in logical pixels. |
| `width` | no | Explicit width for a rendered node, in logical pixels. |
| `min-width` | no | Minimum width for a rendered node, in logical pixels. |
| `grow` | no | `"true"` makes the node use the available width. |

For a notes interface, a grid of cards can be declared directly in XML:

```xml
<screen id="notes" role="screen" label="Notes" layout="column" gap="16">
  <group id="toolbar" role="group" label="Rechercher" layout="row" gap="12" padding="16" wrap="true">
    <component id="query" role="text-input" label="Titre ou contenu" bind="notes.query"
               placeholder="Rechercher une note..." grow="true" />
    <component id="new-note" role="button" label="Nouvelle note" variant="primary" />
  </group>

  <component id="note-grid" role="list" label="Notes" layout="grid" columns="3"
             gap="12" scroll="true" max-height="420" />
</screen>
```

`grid` distributes children by row, while `wrap="true"` keeps a row usable on narrower
windows. Use `scroll` and `max-height` for long collections so the whole screen does not grow
without limit.

### Roles

| Role | Description | Supports `bind` |
|---|---|---|
| `screen` | Top-level page. Multiple screens → sidebar navigation. | no |
| `group` | Bordered card with a title and arbitrary children. | no |
| `dialog` / `view` | Modal overlay. Opened/closed via `view.open` / `view.close`. | no |
| `button` | Clickable button. Triggers actions. | no |
| `text-input` | Single-line text field. | yes |
| `text-area` | Multi-line text field. | yes |
| `checkbox` | Boolean checkbox. | yes — stores `"true"` / `"false"` |
| `toggle` | Boolean toggle switch. | yes — stores `"true"` / `"false"` |
| `select` | Dropdown. Children with `role="option"` are the choices. | yes — stores the selected label |
| `option` | Choice inside a `select`. Only `label` matters. | no |
| `label` | Static or data-bound text. | yes — displays `state[bind]` |
| `heading` | Section title with optional icon. | no |
| `badge` | Accent pill (e.g. a status tag). | yes |
| `chip` | Neutral pill (e.g. a tag or plan name). | no |
| `list` | Bordered container for `list-item` children. | no |
| `list-item` | Row inside a list. Clickable if it has an action. | no |
| `progress` | Progress bar 0–100. | yes — reads a numeric string |
| `slider` | Range slider 0–100. | yes — stores the current value |
| `separator` | Horizontal rule. Set `label=""`. | no |
| `card` | Content card with title and children. | no |
| `image` | Image placeholder with icon and label. | no |

### Actions

An action attaches a handler to a node. When the user clicks or an agent invokes, the Lua function named by `handler` is called.

```xml
<component id="send-btn" role="button" label="Send" icon="send">
  <action
    id="send_message"
    handler="send_message"
    description="Send the current draft to the recipient"
    shortcut="cmd+return">
    <param name="priority" type="string" required="false"
           description="Override priority: low | normal | high | urgent" />
  </action>
</component>
```

`<action>` attributes:

| Attribute | Required | Description |
|---|---|---|
| `id` | yes | Identifier used by agents to call the action. |
| `handler` | yes | Name of the Lua function in `actions.lua`. |
| `description` | no | Natural-language hint shown to agents. |
| `shortcut` | no | Keyboard shortcut: `cmd+return`, `ctrl+s`, `escape`, etc. |

`<param>` attributes:

| Attribute | Required | Description |
|---|---|---|
| `name` | yes | Parameter key passed to the Lua handler in `arguments`. |
| `type` | no | `string` (default), `number`, `boolean`. |
| `required` | no | `"true"` — the runtime rejects calls that omit this parameter. |
| `description` | no | Hint shown to agents. |

Actions without parameters can be written as self-closing: `<action id="..." handler="..." />`.

---

## manifest.yml — appearance and build configuration

Optional. Must sit in the same directory as `app.xml`. Controls window dimensions, visual theme, colours, typography, animation timing, the metadata used by `scrawler build` to produce a distributable binary, and the embedded MCP HTTP server. All values fall back to the defaults when the file is absent or a key is omitted.

### `app` — identity and build metadata

```yaml
app:
  id: "com.example.myapp"        # Used for packaging; falls back to <app id>
  display_name: "My App"         # Window title; falls back to <app name>
  icon: "app-icon.png"           # Path relative to manifest.yml
  version: "1.0.0"
  build: "1.0.0"
  copyright: "Copyright © 2026 Example"
  author: "Example Inc."
```

### `platforms` — platform-specific packaging

```yaml
platforms:
  macos:
    bundle_id: "com.example.myapp"
    category: "public.app-category.productivity"
    minimum_version: "12.0"
  windows:
    app_id: "ExampleMyApp"
    store_category: "Productivity"
  linux:
    desktop_id: "com.example.myapp"
    categories: ["Office", "Utility"]
```

### `window`

| Key | Default | Description |
|---|---|---|
| `width` | `1080` | Initial window width in logical pixels |
| `height` | `720` | Initial window height |
| `min_width` | `640` | Minimum resizable width |
| `min_height` | `480` | Minimum resizable height |
| `resizable` | `true` | Whether the user can resize the window |
| `always_on_top` | `false` | Keep the window above all others |
| `start_maximized` | `false` | Open maximized |
| `start_fullscreen` | `false` | Open fullscreen |

### `theme`

```yaml
theme: "light"   # "light" (default) | "dark" | "system"
```

### `font`

| Key | Default |
|---|---|
| `size_base` | `14` |
| `size_heading` | `18` |
| `size_title` | `24` |
| `size_caption` | `12` |

`family` is reserved for future use; custom fonts are not yet loaded from disk.

### `colors` and `colors_dark`

Both blocks accept the same keys. `colors` is used for light theme, `colors_dark` for dark theme. All values are `#RRGGBB` or `#RRGGBBAA` hex strings.

| Key | Light default | Dark default |
|---|---|---|
| `bg_primary` | `#FAFAFC` | `#0F0F14` |
| `bg_surface` | `#FFFFFF` | `#1A1A24` |
| `bg_elevated` | `#FFFFFF` | `#22222E` |
| `text_primary` | `#1A1A2E` | `#F0F0F5` |
| `text_secondary` | `#6B7080` | `#9A9AAA` |
| `text_on_accent` | `#FFFFFF` | `#FFFFFF` |
| `accent` | `#2D5BE3` | `#5B8DF8` |
| `accent_hover` | `#1E4BD1` | `#7AA5FA` |
| `accent_subtle` | `#EBF0FD` | `#1E293D` |
| `border` | `#E8EAED` | `#2E2E3A` |
| `error_text` | `#DC2626` | `#F87171` |
| `error_bg` | `#FEF2F2` | `#2D1515` |
| `error_border` | `#FECACA` | `#5C2222` |
| `toast_success` | `#059669` | `#34D399` |
| `toast_bg` | `#1A1A2E` | `#1A1A2E` |

### `sidebar`

Sidebar is only rendered when `app.xml` declares more than one `screen`.

| Key | Default |
|---|---|
| `width` | `220` |
| `bg` | `#F3F4F6` |
| `text` | `#4B5063` |
| `text_active` | `#1A1A2E` |
| `item_active_bg` | `#FFFFFF` |

### `dialog`

| Key | Default |
|---|---|
| `bg` | `#FFFFFF` |
| `corner_radius` | `16` |
| `margin` | `28` |

### `input`

| Key | Default |
|---|---|
| `bg` | `#FFFFFF` |
| `border` | `#D1D5DB` |
| `corner_radius` | `8` |

### `spacing`

| Key | Default |
|---|---|
| `content_margin_x` | `40` |
| `content_margin_y` | `32` |
| `titlebar_height` | `32` |

### `corner_radius`

| Key | Default |
|---|---|
| `button` | `10` |
| `card` | `12` |
| `badge` | `6` |

### `animations` — durations in seconds

| Key | Default |
|---|---|
| `toast_duration` | `3.5` |
| `dialog_open` | `0.35` |
| `dialog_close` | `0.2` |
| `screen_transition` | `0.3` |
| `node_appear_stagger` | `0.05` |
| `node_appear_duration` | `0.35` |
| `select_open` | `0.35` |
| `select_close` | `0.2` |
| `select_item_stagger` | `0.06` |
| `select_flash` | `0.25` |

### `density`

```yaml
density: "comfortable"   # "compact" | "comfortable" (default) | "spacious"
```

Scales spacing by `0.8`, `1.0`, or `1.25` respectively.

### `notifications`

| Key | Default | Notes |
|---|---|---|
| `position` | `"bottom-center"` | `"top-right"` \| `"bottom-right"` \| `"bottom-center"` |
| `max_visible` | `3` | Reserved; currently one toast is shown at a time |

### `scrollbar`

| Key | Default |
|---|---|
| `width` | `6` |
| `auto_hide` | `true` |

### `shadows`

| Key | Default |
|---|---|
| `enabled` | `true` |
| `intensity` | `0.08` |

### `hover`

| Key | Default |
|---|---|
| `scale` | `1.0` |
| `transition` | `0.15` |

### `focus_ring`

| Key | Default |
|---|---|
| `color` | `null` (uses accent) |
| `width` | `2` |
| `offset` | `2` |

### `mcp` — embedded HTTP connector

When `port` is set to a non-zero value, `scrawler run` automatically starts an MCP HTTP server on `127.0.0.1:<port>` alongside the native window. The same two tools (`scrawler_get_semantic_tree`, `scrawler_invoke_action`) are available over HTTP. A badge **"Connecteur IA en ligne"** appears in the app's title bar; clicking it shows ready-to-use configuration snippets for every major AI client.

| Key | Default | Description |
|---|---|---|
| `port` | `0` | TCP port for the embedded MCP HTTP server. `0` disables it. |

```yaml
mcp:
  port: 7080
```

The server listens on `127.0.0.1` only (localhost) and is never exposed to the network. The endpoint is `POST http://127.0.0.1:<port>/mcp`.

---

## Lua runtime

`actions.lua` is a sandboxed Lua 5.4 script. Every function you declare becomes a callable handler.

### Handler signature

```lua
function my_handler(arguments, context)
  -- arguments: table of declared <param> values
  -- context:   capabilities table (see below)
  -- return: one effect table, or an array of effect tables
end
```

A handler must return at least one effect. An effect is a plain Lua table with at minimum `effect` (string) and `target` (string) keys.

### Sandbox restrictions

The following standard Lua libraries are **not available**: `io`, `os`, `package`, `debug`, `require`, `load`, `loadfile`, `dofile`, `collectgarbage`. File I/O and process spawning must go through `context.storage` and the effect system respectively.

Available libraries: `table`, `string`, `math`, and all Lua built-ins that do not touch the OS.

---

## Context API

The `context` table is the second argument to every handler. It provides all capabilities as pure functions that return effect tables (or direct values for read-only operations).

### `context.state`

| Function | Returns | Description |
|---|---|---|
| `context.state.get(key)` | `string \| nil` | Read current value of a state key. Direct return, no effect. |
| `context.state.set(key, value)` | effect | Produce a `state.set` effect. The renderer updates its dictionary. |
| `context.state.toggle(key)` | effect | Invert a boolean state key (`"true"` ↔ `"false"`). |

```lua
local current = context.state.get("dark_mode")
return context.state.toggle("dark_mode")
```

### `context.view`

| Function | Returns | Description |
|---|---|---|
| `context.view.open(id, payload)` | effect | Open a `dialog` or `view` node. `payload.state` can pre-set state keys. |
| `context.view.close(id)` | effect | Close a dialog/view with its close animation. |

```lua
return context.view.open("composer", {
  state = { ["draft.recipient"] = arguments.recipient or "" }
})
```

### `context.screen`

| Function | Returns | Description |
|---|---|---|
| `context.screen.navigate(screen_id)` | effect | Switch to another screen with a transition. |

### `context.manifest.node`

Runtime mutations to the visible UI tree. Both the renderer and the MCP server apply these locally.

| Function | Returns | Description |
|---|---|---|
| `context.manifest.node.set_label(id, label)` | effect | Change a node's displayed label. |
| `context.manifest.node.set_visible(id, bool)` | effect | Show or hide a node. |
| `context.manifest.node.set_icon(id, icon_name)` | effect | Change a node's Lucide icon. |

### `context.manifest.select`

| Function | Returns | Description |
|---|---|---|
| `context.manifest.select.set_options(id, options)` | effect | Replace the `option` children of a `select` node. `options` is a string array. |

```lua
return context.manifest.select.set_options("priority-select", {"Low", "Normal", "High", "Urgent"})
```

### `context.notification`

| Function | Returns | Description |
|---|---|---|
| `context.notification.show(message)` | effect | Display a temporary in-app toast banner. |
| `context.notification.os(title, body)` | effect | Send a native OS notification. |

### `context.browser`

| Function | Returns | Description |
|---|---|---|
| `context.browser.open(url)` | effect | Open a URL in the system default browser. |

### `context.clipboard`

| Function | Returns | Description |
|---|---|---|
| `context.clipboard.write(text)` | effect | Write text to the system clipboard. |
| `context.clipboard.read()` | `string` | Read current clipboard text. Direct return. |

### `context.sound`

| Function | Returns | Description |
|---|---|---|
| `context.sound.play(name)` | effect | Play a system sound: `"success"`, `"error"`, or any name (falls back to a default). |

### `context.window`

| Function | Returns | Description |
|---|---|---|
| `context.window.set_title(title)` | effect | Update the window title dynamically. |
| `context.window.set_badge(count)` | effect | Set the Dock badge number (macOS only). `0` clears it. |
| `context.window.minimize()` | effect | Minimize the window. |
| `context.window.close()` | effect | Close the window and exit. |

### `context.file`

| Function | Returns | Description |
|---|---|---|
| `context.file.save(filename, content)` | effect | Open a native "Save As" dialog and write `content` to the chosen path. |

### `context.date`

Date utilities — `os.time` is not available in the sandbox.

| Function | Returns | Description |
|---|---|---|
| `context.date.now()` | `integer` | Current UTC Unix timestamp (seconds since epoch). Direct return. |
| `context.date.format(timestamp, pattern)` | `string` | Format a Unix timestamp using `strftime`-style patterns (e.g. `"%Y-%m-%d"`). Direct return. |

```lua
local ts = context.date.now()
local formatted = context.date.format(ts, "%d %b %Y")
```

### `context.json`

| Function | Returns | Description |
|---|---|---|
| `context.json.encode(table)` | `string` | Serialize a Lua table to a JSON string. Direct return. |
| `context.json.decode(string)` | `table \| nil` | Parse a JSON string into a Lua table. Returns `nil` on parse error. Direct return. |

```lua
local payload = context.json.decode(arguments.raw_json)
local out = context.json.encode({ status = "ok", count = 3 })
```

### `context.form`

| Function | Returns | Description |
|---|---|---|
| `context.form.reset(node_id)` | effect | Clear all `<param>` input fields for a given node's actions. |

### `context.storage`

Persistent key-value store and file sandbox. Data is written to the OS application data directory, namespaced by `app.id` from `app.xml`:
- **macOS**: `~/Library/Application Support/{app.id}/`
- **Windows**: `%APPDATA%\{app.id}\`
- **Linux**: `~/.local/share/{app.id}/`

**KV operations:**

| Function | Returns | Description |
|---|---|---|
| `context.storage.get(key)` | `any \| nil` | Read a persisted JSON value. Direct return. |
| `context.storage.get_all()` | `table` | Return all KV pairs as a table. Direct return. |
| `context.storage.set(key, value)` | effect | Persist a value (any JSON-serialisable type). |
| `context.storage.delete(key)` | effect | Remove a key. |

**File operations (sandboxed to the app data directory):**

| Function | Returns | Description |
|---|---|---|
| `context.storage.file.read(path)` | `string \| nil` | Read a file. Path is relative to the app data dir. Direct return. |
| `context.storage.file.write(path, content)` | effect | Write a file. Parent directories are created automatically. |
| `context.storage.file.delete(path)` | effect | Delete a file. |
| `context.storage.file.list(path)` | `string[]` | List entries in a directory. Direct return. |
| `context.storage.file.mkdir(path)` | effect | Create a directory recursively. |

Path traversal is blocked. Any path containing `..` or starting with `/` returns `nil` / does nothing.

```lua
-- Persist user preferences
local prefs = context.storage.get("prefs") or {}
prefs.theme = "dark"
return context.storage.set("prefs", prefs)
```

### `context.http`

#### `context.http.fetch(url, options)`

Blocking HTTP call. Returns a table directly (no effect produced).

```lua
local resp = context.http.fetch("https://api.example.com/data", {
  method  = "POST",           -- GET (default), POST, PUT, PATCH, DELETE, etc.
  headers = { ["Authorization"] = "Bearer " .. token,
              ["Content-Type"]  = "application/json" },
  body    = context.json.encode({ query = "hello" }),
  timeout = 15,               -- seconds, default 30
})

if resp.ok then
  local data = context.json.decode(resp.body)
  return context.notification.show("Got " .. data.count .. " results")
else
  return context.notification.show("Error " .. resp.status)
end
```

Response table:

| Field | Type | Description |
|---|---|---|
| `status` | `integer` | HTTP status code. `0` on connection error. |
| `body` | `string` | Response body as text. |
| `headers` | `table` | Response headers (string → string). |
| `ok` | `boolean` | `true` if `200 ≤ status < 300`. |
| `error` | `string` | Present only on connection-level failures. |

**Note:** `fetch` blocks the Lua handler thread for the duration of the request. Keep timeouts short for interactive actions.

#### WebSocket

WebSocket connections run in background threads and push messages back as Lua handler invocations.

```lua
-- Open a connection
return context.http.ws.connect("prices", "wss://stream.example.com/live", {
  on_message = "handle_price_update",   -- Lua function called for each message
  on_close   = "handle_ws_closed",      -- Lua function called on disconnect
})

-- Send a message (from any handler)
return context.http.ws.send("prices", context.json.encode({ subscribe = "BTC" }))

-- Close a connection
return context.http.ws.close("prices")
```

`on_message` handler receives `arguments.data` as the raw message string:

```lua
function handle_price_update(arguments, context)
  local msg = context.json.decode(arguments.data)
  return context.state.set("btc.price", tostring(msg.price))
end
```

`on_close` handler receives no arguments:

```lua
function handle_ws_closed(arguments, context)
  return context.notification.show("Connection lost")
end
```

Connection IDs (`"prices"` above) are app-scoped strings. Opening a second connection with the same ID does not close the first — use distinct IDs or explicitly close first.

---

## Effect reference

Handlers return effect tables. The renderer and the MCP server apply them independently. Most effects are produced by `context.*` helpers; this table shows the raw wire format for reference and for writing custom effects.

| `effect` value | `target` | `payload` | Applied by |
|---|---|---|---|
| `state.set` | state key | `{ value: string }` | renderer, (MCP reads state at invoke time) |
| `view.open` | dialog id | `{ state?: { key: val, … } }` | renderer |
| `view.close` | dialog id | `{}` | renderer |
| `screen.navigate` | screen id | `{}` | renderer |
| `manifest.set_label` | node id | `{ label: string }` | renderer + MCP |
| `manifest.set_visible` | node id | `{ visible: bool }` | renderer + MCP |
| `manifest.set_icon` | node id | `{ icon: string }` | renderer + MCP |
| `manifest.set_options` | select id | `{ options: string[] }` | renderer + MCP |
| `form.reset` | node id | `{}` | renderer only |
| `notification.show` | `"notification"` | `{ message: string }` | renderer |
| `notification.os` | `"os"` | `{ title: string, body: string }` | renderer |
| `browser.open` | url | `{}` | renderer |
| `clipboard.write` | `"clipboard"` | `{ text: string }` | renderer |
| `sound.play` | sound name | `{}` | renderer |
| `window.set_title` | `"window"` | `{ title: string }` | renderer |
| `window.set_badge` | `"window"` | `{ count: integer }` | renderer (macOS) |
| `window.minimize` | `"window"` | `{}` | renderer |
| `window.close` | `"window"` | `{}` | renderer |
| `file.save` | filename | `{ content: string }` | renderer |
| `storage.set` | key | `{ value: any }` | renderer + MCP |
| `storage.delete` | key | `{}` | renderer + MCP |
| `storage.file.write` | relative path | `{ content: string }` | renderer + MCP |
| `storage.file.delete` | relative path | `{}` | renderer + MCP |
| `storage.dir.create` | relative path | `{}` | renderer + MCP |
| `http.ws.connect` | conn id | `{ url, on_message, on_close }` | renderer |
| `http.ws.send` | conn id | `{ data: string }` | renderer |
| `http.ws.close` | conn id | `{}` | renderer |

Effects marked **renderer only** are silently ignored by the MCP server. Effects marked **renderer + MCP** are applied in both processes so the agent's next `scrawler_get_semantic_tree` call reflects the mutations.

Multiple effects: return a Lua array.

```lua
return {
  context.state.set("sent", "true"),
  context.view.close("composer"),
  context.notification.show("Sent"),
}
```

---

## MCP interface

The runtime exposes two JSON-RPC tools. There are two ways to reach them:

| Mode | How to start | Transport | Who uses it |
|---|---|---|---|
| **stdio** | `scrawler serve [app.xml]` | newline-delimited JSON on stdin/stdout | Claude Code CLI, CI pipelines, any stdio MCP host |
| **HTTP** | `scrawler run` + `mcp.port` in `manifest.yml` | `POST http://127.0.0.1:<port>/mcp` | Claude Desktop, ChatGPT, Gemini, Cursor, VS Code, any HTTP MCP host |

Both modes share the same two tools and the same Lua runtime.

### Connecting an AI client (HTTP mode)

Set `mcp.port` in `manifest.yml` and launch the app. Then point your client at `http://127.0.0.1:<port>/mcp`:

**Claude Code CLI**
```bash
claude mcp add MyApp --transport http http://127.0.0.1:7080/mcp
```

**Claude Desktop** — edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):
```json
{
  "mcpServers": {
    "my-app": { "type": "http", "url": "http://127.0.0.1:7080/mcp" }
  }
}
```
Restart Claude Desktop after saving.

**ChatGPT Desktop** — Settings → Plugins → MCP → Add server → enter the URL.

**Gemini / Google AI Studio** — Extensions → Add MCP tool (HTTP) → enter the URL.

**Cursor / VS Code Copilot** — add to workspace `.vscode/mcp.json`:
```json
{
  "mcp": {
    "servers": {
      "my-app": { "type": "http", "url": "http://127.0.0.1:7080/mcp" }
    }
  }
}
```

When started with `scrawler serve`, the process exposes two JSON-RPC tools over stdin/stdout.

### `scrawler_get_semantic_tree`

Returns the full application tree as structured text. Call this once before invoking any actions to discover node IDs and action IDs.

```json
{ "node_id": "compose", "role": "button", "label": "Compose", "icon": "pencil",
  "actions": [{ "id": "compose_message", "handler": "compose_message",
                "description": "Open a new email draft",
                "parameters": [{ "name": "recipient", "type": "string",
                                 "required": false, "description": "…" }] }] }
```

### `scrawler_invoke_action`

```json
{
  "node_id":   "compose",
  "action_id": "compose_message",
  "arguments": { "recipient": "alice@example.com" }
}
```

Arguments are validated against the declared `<param>` types and `required` flags before the Lua handler runs. On success the response contains the list of effects that were produced and forwarded to the renderer.

If the renderer is not running, effects are still applied to the MCP server's local copy of the tree and a `warning: ui offline` line is appended to the response.

---

## CLI commands

| Command | Description |
|---|---|
| `scrawler run [app.xml]` | Open the native window. |
| `scrawler serve [app.xml]` | Start the MCP server on stdin/stdout. |
| `scrawler inspect [app.xml]` | Print the parsed tree as JSON and exit. |
| `scrawler build [app.xml] [target]` | Bundle into a native executable. |

If `app.xml` is omitted, the runtime looks for it in the current working directory or (when run from a `.app` bundle) in the adjacent `Resources/` directory.

---

## Directory layout

```
my-app/
├── app.xml          # UI tree, state, actions
├── actions.lua      # Lua handlers
└── manifest.yml     # optional — appearance, window, theme, build metadata
```

Persistent data is stored separately in the OS data directory and is never inside the app folder.

---

## Constraints and non-obvious behaviour

- **State is always a string internally.** `context.state.get` always returns a string (or nil). To work with numbers, use `tonumber()`. Booleans are `"true"` / `"false"`.
- **`context.state.get` snapshots the state at handler call time.** If an earlier effect in the same invocation changed a key, `get` will not reflect it — the snapshot was taken before any effects ran.
- **Effects are applied after the handler returns.** A handler that reads a key after emitting an effect that would change it will read the old value.
- **`manifest.set_visible` does not remove the node from the MCP tree.** Agents will still see hidden nodes in `scrawler_get_semantic_tree`; they just are not rendered.
- **`context.http.fetch` is blocking.** Do not use it for long-running operations triggered by UI buttons without setting a short `timeout`. The Lua VM is single-threaded.
- **WebSocket `on_message` / `on_close` handlers are called on the renderer's main loop.** They must return effects normally. They should not call `http.fetch` with long timeouts.
- **`context.dialog.confirm` does not exist.** Use application components (`dialog`, `view`, buttons) to build confirmation flows. This constraint is intentional.
- **`os`, `io`, `package`, `debug`, `require` are not available.** The Lua sandbox is intentionally narrow. External libraries cannot be loaded.
- **No persistent connections survive a process restart.** WebSocket connections are in-process; restarting the renderer or MCP server drops them.
- **`manifest.yml` is ignored by `scrawler serve`.** Theme, window size, and build metadata have no effect on the MCP interface.
