use crate::config;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FALLBACK_ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

pub fn bundle(app_xml_path: &str, target_os: Option<&str>) -> Result<PathBuf, String> {
    let app_path = Path::new(app_xml_path);
    let app_dir = match app_path.parent() {
        Some(p) if p.as_os_str().is_empty() => Path::new("."),
        Some(p) => p,
        None => Path::new("."),
    };

    if !app_path.exists() {
        return Err(format!("File not found: {app_xml_path}"));
    }

    let cfg = config::load_config(app_dir);

    let display_name = cfg
        .display_name
        .as_deref()
        .unwrap_or("ScrawlerApp");
    let bundle_id = cfg
        .macos_bundle_id
        .as_deref()
        .or(cfg.app_id.as_deref())
        .unwrap_or("com.scrawler.app");

    let os = target_os.unwrap_or(std::env::consts::OS);

    let out_dir = app_dir.join("dist");
    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Cannot create dist directory: {e}"))?;

    // Use the currently-running binary as the runtime — no recompile needed.
    // For cross-compilation, fall back to cargo build.
    let runtime = if target_os.is_none() || target_os == Some(std::env::consts::OS) {
        std::env::current_exe()
            .map_err(|e| format!("Cannot locate current executable: {e}"))?
    } else {
        compile_release(Some(target_os.unwrap()))?
    };

    match os {
        "macos" => bundle_macos(app_dir, &out_dir, display_name, bundle_id, &cfg, &runtime),
        "windows" => bundle_windows(app_dir, &out_dir, display_name, &runtime),
        "linux" => bundle_linux(app_dir, &out_dir, display_name, &cfg, &runtime),
        other => Err(format!("Unsupported target: {other}. Use: macos, windows, linux")),
    }
}

fn compile_release(target: Option<&str>) -> Result<PathBuf, String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--release");
    if let Some(t) = target {
        cmd.arg("--target").arg(t);
    }

    let status = cmd
        .status()
        .map_err(|e| format!("Failed to run cargo build: {e}"))?;

    if !status.success() {
        return Err("cargo build --release failed".into());
    }

    let target_dir = if let Some(t) = target {
        PathBuf::from("target").join(t).join("release")
    } else {
        PathBuf::from("target").join("release")
    };

    let binary_name = if cfg!(target_os = "windows") || target.is_some_and(|t| t.contains("windows")) {
        "scrawler.exe"
    } else {
        "scrawler"
    };

    let binary = target_dir.join(binary_name);
    if !binary.exists() {
        return Err(format!("Expected binary not found at {}", binary.display()));
    }

    Ok(binary)
}

fn bundle_macos(
    app_dir: &Path,
    out_dir: &Path,
    display_name: &str,
    bundle_id: &str,
    cfg: &config::AppConfig,
    binary: &Path,
) -> Result<PathBuf, String> {

    let app_bundle = out_dir.join(format!("{display_name}.app"));
    let contents = app_bundle.join("Contents");
    let macos_dir = contents.join("MacOS");
    let resources = contents.join("Resources");

    // Clean previous bundle
    if app_bundle.exists() {
        fs::remove_dir_all(&app_bundle)
            .map_err(|e| format!("Cannot clean previous bundle: {e}"))?;
    }

    fs::create_dir_all(&macos_dir)
        .map_err(|e| format!("Cannot create MacOS dir: {e}"))?;
    fs::create_dir_all(&resources)
        .map_err(|e| format!("Cannot create Resources dir: {e}"))?;

    fs::copy(&binary, macos_dir.join(display_name))
        .map_err(|e| format!("Cannot copy binary: {e}"))?;

    // Copy app files (xml, lua, manifest.yml, etc.) into Resources
    copy_app_resources(app_dir, &resources)?;

    // Convert icon to .icns if possible
    let icon_file = convert_icon_to_icns(app_dir, &resources, cfg)?;

    let version = cfg.version.as_deref().unwrap_or("1.0.0");
    let build_number = cfg.build.as_deref().unwrap_or(version);
    let copyright = cfg.copyright.as_deref().unwrap_or("");
    let min_sys = cfg.macos_minimum_version.as_deref().unwrap_or("11.0");
    let category = cfg.macos_category.as_deref().unwrap_or("public.app-category.utilities");

    let mut plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>{display_name}</string>
    <key>CFBundleIdentifier</key>
    <string>{bundle_id}</string>
    <key>CFBundleName</key>
    <string>{display_name}</string>
    <key>CFBundleDisplayName</key>
    <string>{display_name}</string>
    <key>CFBundleVersion</key>
    <string>{build_number}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleIconFile</key>
    <string>{icon_file}</string>
    <key>LSMinimumSystemVersion</key>
    <string>{min_sys}</string>
    <key>LSApplicationCategoryType</key>
    <string>{category}</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>"#
    );

    if !copyright.is_empty() {
        plist.push_str(&format!(
            "\n    <key>NSHumanReadableCopyright</key>\n    <string>{copyright}</string>"
        ));
    }

    plist.push_str(
        "\n</dict>\n</plist>\n"
    );

    fs::write(contents.join("Info.plist"), plist)
        .map_err(|e| format!("Cannot write Info.plist: {e}"))?;

    // Make binary executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        fs::set_permissions(macos_dir.join(display_name), perms)
            .map_err(|e| format!("Cannot chmod binary: {e}"))?;
    }

    eprintln!("Bundled: {}", app_bundle.display());

    // Generate .dmg
    let dmg_path = create_dmg(out_dir, &app_bundle, display_name, version)?;
    eprintln!("DMG: {}", dmg_path.display());

    Ok(app_bundle)
}

fn create_dmg(
    out_dir: &Path,
    app_bundle: &Path,
    display_name: &str,
    version: &str,
) -> Result<PathBuf, String> {
    let dmg_name = format!("{display_name}-{version}.dmg");
    let dmg_path = out_dir.join(&dmg_name);

    if dmg_path.exists() {
        fs::remove_file(&dmg_path)
            .map_err(|e| format!("Cannot remove old DMG: {e}"))?;
    }

    // Create a temporary directory with .app + Applications symlink
    let staging = out_dir.join(".dmg-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|e| format!("Cannot clean DMG staging: {e}"))?;
    }
    fs::create_dir_all(&staging)
        .map_err(|e| format!("Cannot create DMG staging: {e}"))?;

    // Copy .app into staging
    let staged_app = staging.join(format!("{display_name}.app"));
    copy_dir_recursive(app_bundle, &staged_app)?;

    // Create symlink to /Applications
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .map_err(|e| format!("Cannot create Applications symlink: {e}"))?;

    let status = Command::new("hdiutil")
        .args([
            "create",
            "-volname", display_name,
            "-srcfolder", staging.to_str().unwrap_or_default(),
            "-ov",
            "-format", "UDZO",
            dmg_path.to_str().unwrap_or_default(),
        ])
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Cannot run hdiutil: {e}"))?;

    let _ = fs::remove_dir_all(&staging);

    if !status.success() {
        return Err("hdiutil create failed".into());
    }

    Ok(dmg_path)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Cannot create {}: {e}", dst.display()))?;
    for entry in fs::read_dir(src).map_err(|e| format!("Cannot read {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("Cannot read entry: {e}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Cannot copy {}: {e}", src_path.display()))?;
        }
    }
    Ok(())
}

fn convert_icon_to_icns_from_path(
    png_path: &Path,
    resources: &Path,
) -> Result<String, String> {
    let iconset_dir = resources.join("AppIcon.iconset");
    fs::create_dir_all(&iconset_dir)
        .map_err(|e| format!("Cannot create iconset dir: {e}"))?;

    let sizes: &[(u32, &str)] = &[
        (16, "icon_16x16.png"),
        (32, "icon_16x16@2x.png"),
        (32, "icon_32x32.png"),
        (64, "icon_32x32@2x.png"),
        (128, "icon_128x128.png"),
        (256, "icon_128x128@2x.png"),
        (256, "icon_256x256.png"),
        (512, "icon_256x256@2x.png"),
        (512, "icon_512x512.png"),
        (1024, "icon_512x512@2x.png"),
    ];

    for (size, name) in sizes {
        let output = iconset_dir.join(name);
        let status = Command::new("sips")
            .args([
                "-z", &size.to_string(), &size.to_string(),
                png_path.to_str().unwrap_or_default(),
                "--out", output.to_str().unwrap_or_default(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if status.is_err() || !status.unwrap().success() {
            let _ = fs::remove_dir_all(&iconset_dir);
            return Ok("app-icon.png".to_string());
        }
    }

    let icns_path = resources.join("AppIcon.icns");
    let status = Command::new("iconutil")
        .args([
            "-c", "icns",
            iconset_dir.to_str().unwrap_or_default(),
            "-o", icns_path.to_str().unwrap_or_default(),
        ])
        .status();

    let _ = fs::remove_dir_all(&iconset_dir);

    match status {
        Ok(s) if s.success() => Ok("AppIcon.icns".to_string()),
        _ => Ok("app-icon.png".to_string()),
    }
}

fn convert_icon_to_icns(
    app_dir: &Path,
    resources: &Path,
    cfg: &config::AppConfig,
) -> Result<String, String> {
    let icon_source = cfg.icon_path.as_deref().unwrap_or("app-icon.png");
    let source_path = app_dir.join(icon_source);

    if !source_path.exists() {
        let fallback_path = resources.join("app-icon.png");
        fs::write(&fallback_path, FALLBACK_ICON_PNG)
            .map_err(|e| format!("Cannot write fallback icon: {e}"))?;
        return convert_icon_to_icns_from_path(&fallback_path, resources);
    }

    // If already .icns, just copy
    if icon_source.ends_with(".icns") {
        fs::copy(&source_path, resources.join(icon_source))
            .map_err(|e| format!("Cannot copy icon: {e}"))?;
        return Ok(icon_source.to_string());
    }

    // Convert webp to PNG first (sips doesn't support webp input)
    let effective_source = if icon_source.ends_with(".webp") {
        let tmp_png = resources.join("_icon_converted.png");
        let status = Command::new("sips")
            .args([
                "-s", "format", "png",
                source_path.to_str().unwrap_or_default(),
                "--out", tmp_png.to_str().unwrap_or_default(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if status.is_err() || !status.unwrap().success() {
            eprintln!("Warning: could not convert webp to png, copying as-is");
            fs::copy(&source_path, resources.join(icon_source))
                .map_err(|e| format!("Cannot copy icon: {e}"))?;
            return Ok(icon_source.to_string());
        }
        tmp_png
    } else {
        source_path.clone()
    };

    // Use sips + iconutil to convert image → .icns (macOS only)
    // Supports: png, jpeg, jpg, tiff, webp (via conversion above)
    let iconset_dir = resources.join("AppIcon.iconset");
    fs::create_dir_all(&iconset_dir)
        .map_err(|e| format!("Cannot create iconset dir: {e}"))?;

    let sizes: &[(u32, &str)] = &[
        (16, "icon_16x16.png"),
        (32, "icon_16x16@2x.png"),
        (32, "icon_32x32.png"),
        (64, "icon_32x32@2x.png"),
        (128, "icon_128x128.png"),
        (256, "icon_128x128@2x.png"),
        (256, "icon_256x256.png"),
        (512, "icon_256x256@2x.png"),
        (512, "icon_512x512.png"),
        (1024, "icon_512x512@2x.png"),
    ];

    for (size, name) in sizes {
        let output = iconset_dir.join(name);
        let status = Command::new("sips")
            .args([
                "-z", &size.to_string(), &size.to_string(),
                effective_source.to_str().unwrap_or_default(),
                "--out", output.to_str().unwrap_or_default(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if status.is_err() || !status.unwrap().success() {
            eprintln!("Warning: sips not available, copying PNG as-is");
            fs::copy(&source_path, resources.join(icon_source))
                .map_err(|e| format!("Cannot copy icon: {e}"))?;
            let _ = fs::remove_dir_all(&iconset_dir);
            return Ok(icon_source.to_string());
        }
    }

    let icns_path = resources.join("AppIcon.icns");
    let status = Command::new("iconutil")
        .args([
            "-c", "icns",
            iconset_dir.to_str().unwrap_or_default(),
            "-o", icns_path.to_str().unwrap_or_default(),
        ])
        .status();

    let _ = fs::remove_dir_all(&iconset_dir);

    match status {
        Ok(s) if s.success() => Ok("AppIcon.icns".to_string()),
        _ => {
            eprintln!("Warning: iconutil failed, copying PNG as-is");
            fs::copy(&source_path, resources.join(icon_source))
                .map_err(|e| format!("Cannot copy icon: {e}"))?;
            Ok(icon_source.to_string())
        }
    }
}

fn bundle_windows(
    app_dir: &Path,
    out_dir: &Path,
    display_name: &str,
    binary: &Path,
) -> Result<PathBuf, String> {

    let bundle_dir = out_dir.join(format!("{display_name}-win64"));
    if bundle_dir.exists() {
        fs::remove_dir_all(&bundle_dir)
            .map_err(|e| format!("Cannot clean previous bundle: {e}"))?;
    }
    fs::create_dir_all(&bundle_dir)
        .map_err(|e| format!("Cannot create bundle dir: {e}"))?;

    let exe_name = format!("{display_name}.exe");
    fs::copy(&binary, bundle_dir.join(&exe_name))
        .map_err(|e| format!("Cannot copy binary: {e}"))?;

    copy_app_resources(app_dir, &bundle_dir)?;

    eprintln!("Bundled: {}", bundle_dir.display());
    Ok(bundle_dir)
}

fn bundle_linux(
    app_dir: &Path,
    out_dir: &Path,
    display_name: &str,
    cfg: &config::AppConfig,
    binary: &Path,
) -> Result<PathBuf, String> {

    let appdir = out_dir.join(format!("{display_name}.AppDir"));
    if appdir.exists() {
        fs::remove_dir_all(&appdir)
            .map_err(|e| format!("Cannot clean previous bundle: {e}"))?;
    }

    let usr_bin = appdir.join("usr").join("bin");
    let usr_share = appdir.join("usr").join("share");

    fs::create_dir_all(&usr_bin)
        .map_err(|e| format!("Cannot create bin dir: {e}"))?;
    fs::create_dir_all(&usr_share)
        .map_err(|e| format!("Cannot create share dir: {e}"))?;

    fs::copy(&binary, usr_bin.join(display_name))
        .map_err(|e| format!("Cannot copy binary: {e}"))?;

    copy_app_resources(app_dir, &usr_share)?;

    let desktop_id = cfg.linux_desktop_id.as_deref().unwrap_or(display_name);
    let categories = if cfg.linux_categories.is_empty() {
        "Utility;".to_string()
    } else {
        cfg.linux_categories.join(";") + ";"
    };
    let desktop_entry = format!(
        "[Desktop Entry]\nName={display_name}\nExec={display_name}\nType=Application\nStartupWMClass={desktop_id}\nCategories={categories}\n"
    );
    fs::write(appdir.join(format!("{display_name}.desktop")), desktop_entry)
        .map_err(|e| format!("Cannot write .desktop: {e}"))?;

    let apprun = format!("#!/bin/sh\nHERE=$(dirname \"$(readlink -f \"$0\")\")\nexec \"$HERE/usr/bin/{display_name}\" \"$HERE/usr/share/app.xml\" \"$@\"\n");
    fs::write(appdir.join("AppRun"), &apprun)
        .map_err(|e| format!("Cannot write AppRun: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        fs::set_permissions(appdir.join("AppRun"), perms)
            .map_err(|e| format!("Cannot chmod AppRun: {e}"))?;
    }

    eprintln!("Bundled: {}", appdir.display());
    eprintln!("To create an AppImage, run: appimagetool {}", appdir.display());
    Ok(appdir)
}

fn copy_app_resources(app_dir: &Path, dest: &Path) -> Result<(), String> {
    for entry in fs::read_dir(app_dir).map_err(|e| format!("Cannot read app dir: {e}"))? {
        let entry = entry.map_err(|e| format!("Cannot read entry: {e}"))?;
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap();
            // Skip dist directory artifacts
            if filename == "dist" {
                continue;
            }
            fs::copy(&path, dest.join(filename))
                .map_err(|e| format!("Cannot copy {}: {e}", path.display()))?;
        }
    }
    Ok(())
}
