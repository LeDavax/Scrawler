//! Point d'entrée en ligne de commande du premier outil Scrawler.

use scrawler::bundle;
use scrawler::manifest::parse_manifest;
use scrawler::mcp::run_stdio_server;
use scrawler::renderer::run_native_app;
use scrawler::runtime::LuaRuntime;
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    let command = arguments.next();
    let path = arguments.next();

    match (command.as_deref(), path) {
        (Some("inspect"), Some(path)) => inspect(&path),
        (Some("inspect"), None) => inspect(&resolve_app_xml_or_exit()),
        (Some("serve"), Some(path)) => serve(&path),
        (Some("serve"), None) => serve(&resolve_app_xml_or_exit()),
        (Some("run"), Some(path)) => run(&path),
        (Some("run"), None) => run(&resolve_app_xml_or_exit()),
        (Some("build"), Some(path)) => bundle_app(&path, arguments.next()),
        (Some("build"), None) => bundle_app(&resolve_app_xml_or_exit(), arguments.next()),
        (Some("bundle"), Some(path)) => bundle_app(&path, arguments.next()),
        (Some("bundle"), None) => bundle_app(&resolve_app_xml_or_exit(), arguments.next()),
        (None, None) => {
            if let Some(adjacent) = find_adjacent_app_xml() {
                run(&adjacent)
            } else {
                print_usage();
                ExitCode::from(2)
            }
        }
        _ => {
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn find_adjacent_app_xml() -> Option<String> {
    // Check current working directory first (user running `scrawler` from their app folder)
    let cwd_candidate = Path::new("app.xml");
    if cwd_candidate.exists() {
        return Some(cwd_candidate.to_string_lossy().into_owned());
    }

    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;

    // macOS .app bundle: binary is in Contents/MacOS/, resources in Contents/Resources/
    let resources_candidate = dir.join("../Resources/app.xml");
    if resources_candidate.exists() {
        return Some(resources_candidate.to_string_lossy().into_owned());
    }

    // Flat layout (Windows/Linux): app.xml next to the binary
    let candidate = dir.join("app.xml");
    if candidate.exists() {
        return Some(candidate.to_string_lossy().into_owned());
    }

    None
}

fn resolve_app_xml_or_exit() -> String {
    if let Some(path) = find_adjacent_app_xml() {
        return path;
    }
    eprintln!("Error: no app.xml found in current directory.");
    eprintln!("Run this command from your Scrawler app folder, or pass a path explicitly.");
    std::process::exit(2);
}

fn print_usage() {
    eprintln!("Usage: scrawler <command> [path-to-app.xml]");
    eprintln!("\nCommands:");
    eprintln!("  run      Opens the native application rendered from the manifest.");
    eprintln!("  build    Packages the app as a native executable (.app / .exe / AppDir).");
    eprintln!("  serve    Starts a local MCP server for the manifest.");
    eprintln!("  inspect  Reads a manifest and prints its semantic tree as JSON.");
    eprintln!("\nIf no path is given, looks for app.xml in the current directory.");
}

fn run(path: &str) -> ExitCode {
    let (manifest, runtime) = match load_application(path) {
        Ok(application) => application,
        Err(error) => {
            eprintln!("Could not start native application: {error}");
            return ExitCode::FAILURE;
        }
    };
    let app_dir = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
    match run_native_app(manifest, runtime, app_dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Native application stopped with an error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn serve(path: &str) -> ExitCode {
    let (manifest, runtime) = match load_application(path) {
        Ok(application) => application,
        Err(error) => {
            eprintln!("Could not start MCP server: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = run_stdio_server(manifest, runtime) {
        eprintln!("MCP server stopped with an I/O error: {error}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn load_application(path: &str) -> Result<(scrawler::manifest::AppManifest, LuaRuntime), String> {
    let xml =
        fs::read_to_string(path).map_err(|error| format!("Could not read {path}: {error}"))?;
    let manifest =
        parse_manifest(&xml).map_err(|error| format!("Manifest validation failed: {error}"))?;
    let actions_path = match manifest.actions.as_deref() {
        Some(relative_path) => Path::new(path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(relative_path),
        None => return Err("<app> requires an `actions` attribute".into()),
    };
    let actions_source = fs::read_to_string(&actions_path).map_err(|error| {
        format!(
            "Could not read Lua actions file {}: {error}",
            actions_path.display()
        )
    })?;
    let runtime = LuaRuntime::from_source(&actions_source, &actions_path.display().to_string())
        .map_err(|error| format!("Lua runtime setup failed: {error}"))?;
    Ok((manifest, runtime))
}

fn bundle_app(path: &str, target: Option<String>) -> ExitCode {
    match bundle::bundle(path, target.as_deref()) {
        Ok(output) => {
            eprintln!("Done: {}", output.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Bundle failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn inspect(path: &str) -> ExitCode {
    let xml = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            eprintln!("Could not read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };

    let manifest = match parse_manifest(&xml) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("Manifest validation failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    match serde_json::to_string_pretty(&manifest) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Could not serialize semantic tree: {error}");
            ExitCode::FAILURE
        }
    }
}
