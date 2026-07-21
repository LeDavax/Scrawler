# Scrawler Semantical Runtime (made for OpenAI build week)

# Notes (by me)
Hello, as I'm only 15 and French (it's my mum who signed up on devpost) I don't speak really good English, that's why it's an AI which wrote the next section `README.md (by llm)`. I wrote some notes myself, there might be many mistakes but now you know why.

## This project
My project is in the "Developper tools" category. My idea is that it's absurd to make the agents take a screenshot, analyze the screenshot and use an api to click on a pixel, only to click on ONE SINGLE button. That's why I made Scrawler Semantic Runtime. The goal is to separate the gui and the logic in order to make my system call the functions on click, on hover, on input, etc. instead of that the functions detect theses events which allows human to interact with the app through a gui and an agent both through a mcp server. (technical details explained by ai in the following sections)

## My project
My final goal is to make a new OS called "ScrawlerOS" where human and agent can interact with everything together, and `Scrawler Semantical Runtime` is a part of this projetc, but as I had only 2 days for this hackaton (yes, I discovered the OpenAI build week a litle bit too late) I "only" made Scrawler Semantical Runtime.

## My usage of GPT-5.6 Terra
As I missed the start of the competition I couldn't get the codex free credits and I'm in free plan so my usage of codex was really limited. I used it to make the structure and the start of the project and it made strong bases, then I continued with an other llm.

## Test it
You can watch my demo video and the setup instructions are in the following sections. The example in the repo is not very good so I recommend that you give the DEVELOPER.md file to en ai, ask it to make an app and test it by running `scrawler run` (after you have installed the binary file as it is explained by ai in the following sections).

---

# README.md (by llm)

Scrawler is a local runtime for semantic, agent-friendly applications.

An app is described in XML, implemented in Lua, and exposed simultaneously as a native window and through MCP. The goal is to let an agent work against a declared semantic tree instead of screen scraping or ad hoc integrations.

## What Scrawler does

Scrawler reads an `app.xml` manifest, validates it, and builds a semantic tree made of `screen`, `view`, `group`, `component`, and `dialog` nodes.

Each declared action points to a Lua handler in `actions.lua`. The handler returns structured effects, and the runtime applies those effects to the native UI or the MCP server’s local copy of the app state.

The repository includes a complete example in:

- [`examples/mail/app.xml`](./examples/mail/app.xml)
- [`examples/mail/actions.lua`](./examples/mail/actions.lua)

## Project structure

```text
my-app/
├── app.xml
├── actions.lua
└── manifest.yml   # optional
```

- `app.xml` defines the semantic UI tree, runtime state, and actions.
- `actions.lua` contains the sandboxed Lua handlers.
- `manifest.yml` is optional and controls native appearance, packaging metadata, and the embedded HTTP MCP port.

## Installation

### macOS (Apple Silicon)

```bash
curl -fsSL https://raw.githubusercontent.com/LeDavax/Scrawler/main/install.sh | sh
```

Or manually:

```bash
curl -L https://github.com/LeDavax/Scrawler/releases/latest/download/scrawler-darwin-aarch64.tar.gz | tar -xz
sudo mv scrawler /usr/local/bin/scrawler
xattr -d com.apple.quarantine /usr/local/bin/scrawler
```

### Linux (x86_64)

```bash
curl -fsSL https://raw.githubusercontent.com/LeDavax/Scrawler/main/install.sh | sh
```

Or manually:

```bash
curl -L https://github.com/LeDavax/Scrawler/releases/latest/download/scrawler-linux-x86_64.tar.gz | tar -xz
sudo mv scrawler /usr/local/bin/scrawler
```

### Windows (x86_64)

Download [`scrawler-windows-x86_64.zip`](https://github.com/LeDavax/Scrawler/releases/latest/download/scrawler-windows-x86_64.zip), extract it, and move `scrawler.exe` to a folder in your `PATH`.

### Windows (ARM64)

Download [`scrawler-windows-aarch64.zip`](https://github.com/LeDavax/Scrawler/releases/latest/download/scrawler-windows-aarch64.zip), extract it, and move `scrawler.exe` to a folder in your `PATH`.

## Usage

All commands work with or without a path. If no path is given, Scrawler looks for `app.xml` in the current directory.

```bash
cd my-app/

scrawler run        # open the native app
scrawler build      # package as .app / .exe / AppDir
scrawler serve      # start an MCP server over stdio
scrawler inspect    # print the semantic tree as JSON
```

Or pass a path explicitly:

```bash
scrawler run path/to/app.xml
```

### Start the MCP server over stdio

Expose the app to an MCP host over stdin/stdout JSON-RPC:

```bash
scrawler serve
```

This mode exposes two tools:

- `scrawler_get_semantic_tree`
- `scrawler_invoke_action`

Recommended flow:

1. Call `scrawler_get_semantic_tree`.
2. Read the relevant `node_id` and `action_id` values.
3. Call `scrawler_invoke_action` with validated arguments.

If the native app is running at the same time, effects can be forwarded to it through the local IPC bridge. If it is not running, the MCP server still applies effects to its own local state and returns a `ui offline` warning.

## How it works

1. `app.xml` is parsed and validated.
2. The semantic tree is built.
3. Lua handlers are loaded from `actions.lua`.
4. A handler receives `arguments` and a restricted `context`.
5. The handler returns one or more JSON-compatible effects.
6. The renderer or MCP server applies those effects locally.

## Lua runtime

The Lua runtime is intentionally restricted.

Available standard libraries:

- `table`
- `string`
- `math`

Unavailable standard libraries:

- `io`
- `os`
- `package`
- `debug`

Handlers cannot spawn processes or load arbitrary code. Side effects go through the effect system and `context.storage`.

## Context API

The runtime exposes a generic context object to Lua handlers.

### State

- `context.state.get(key)`
- `context.state.set(key, value)`
- `context.state.toggle(key)`

### UI and navigation

- `context.view.open(id, payload)`
- `context.view.close(id)`
- `context.screen.navigate(screen_id)`

### Semantic mutations

- `context.manifest.node.set_label(id, label)`
- `context.manifest.node.set_visible(id, visible)`
- `context.manifest.node.set_icon(id, icon_name)`
- `context.manifest.select.set_options(id, options)`

### Notifications and OS actions

- `context.notification.show(message)`
- `context.notification.os(title, body)`
- `context.browser.open(url)`
- `context.clipboard.write(text)`
- `context.clipboard.read()`
- `context.sound.play(name)`
- `context.window.set_title(title)`
- `context.window.set_badge(count)`
- `context.window.minimize()`
- `context.window.close()`
- `context.file.save(filename, content)`

### Data and utilities

- `context.date.now()`
- `context.date.format(timestamp, pattern)`
- `context.json.encode(table)`
- `context.json.decode(string)`
- `context.form.reset(node_id)`

### Persistent storage

Storage is namespaced by `app.id` and stored in the OS application data directory.

- `context.storage.get(key)`
- `context.storage.get_all()`
- `context.storage.set(key, value)`
- `context.storage.delete(key)`
- `context.storage.file.read(path)`
- `context.storage.file.write(path, content)`
- `context.storage.file.delete(path)`
- `context.storage.file.list(path)`
- `context.storage.file.mkdir(path)`

### HTTP

- `context.http.fetch(url, options)`
- `context.http.fetch_async(url, options, callback)`
- `context.http.ws.connect(id, url, options)`
- `context.http.ws.send(id, data)`
- `context.http.ws.close(id)`

## Bundling

`scrawler build` packages the app for the current target or a target you pass explicitly.

```bash
scrawler build examples/mail/app.xml
```

Target-specific output:

- macOS: `.app` bundle plus `.dmg`
- Windows: a bundled folder containing the `.exe`
- Linux: an `.AppDir` layout, ready for AppImage tooling

## Example app

The mail example demonstrates the full model:

- a semantic inbox screen
- a compose dialog
- fields bound to local state
- a select control populated from declared options
- Lua handlers that open and close views, update state, and trigger notifications

Example handler:

```lua
function compose_message(arguments, context)
  return context.view.open("composer", {
    state = { ["draft.recipient"] = arguments.recipient or "" }
  })
end
```

## Useful commands

```bash
scrawler run     [app.xml]
scrawler build   [app.xml]
scrawler serve   [app.xml]
scrawler inspect [app.xml]
```

If no path is provided, Scrawler looks for `app.xml` in the current directory.
