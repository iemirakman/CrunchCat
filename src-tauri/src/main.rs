// Prevents additional console window on Windows in release, DO NOT REMOVE!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::{self, File};
use std::process::Command;
use std::io;
use std::path::{Path, PathBuf};
use tauri::{Manager, RunEvent};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const ARCHIVE_EXTENSIONS: [&str; 3] = ["zip", "rar", "7z"];

/// Process files dropped onto the app's Dock/Desktop icon (macOS).
/// Extracts or compresses silently, regardless of window focus.
fn handle_icon_drop(urls: &[tauri::Url]) {
    for url in urls {
        if let Ok(path) = url.to_file_path() {
            if let Err(e) = process_path(&path) {
                eprintln!("CrunchCat icon-drop error: {e}");
            }
        }
    }
}

/// Returns the path of the "first run answered" state file. Existence means
/// the user already chose Yes/No for the Desktop shortcut banner.
fn state_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Failed to resolve config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {e}"))?;
    Ok(dir.join("first_run_done"))
}

/// Check whether the shortcut question has already been answered.
#[tauri::command]
fn shortcut_state(app: tauri::AppHandle) -> bool {
    state_file(&app).map(|p| p.exists()).unwrap_or(false)
}

/// Create the Desktop shortcut, then return immediately. The state is
/// marked answered right away and the actual (slow) bundle copy is moved
/// to a background thread so the frontend can close the window instantly.
#[tauri::command]
fn create_desktop_shortcut(app: tauri::AppHandle) -> Result<(), String> {
    fs::write(state_file(&app)?, "done").map_err(|e| format!("Failed to save state: {e}"))?;

    let resources = app
        .path()
        .resource_dir()
        .map_err(|e| format!("Failed to resolve bundle resources: {e}"))?;
    let bundle_dir = resources
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| "Unable to resolve app bundle path".to_string())?
        .to_path_buf();

    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let desktop = PathBuf::from(home).join("Desktop");

    // Copy runs off the IPC thread; the window can close immediately while
    // the process keeps copying the bundle in the background.
    std::thread::spawn(move || {
        if let Err(e) = copy_bundle_to_desktop(&bundle_dir, &desktop) {
            eprintln!("CrunchCat shortcut copy error: {e}");
        }
    });

    // Forcefully hide the window at the OS level so it vanishes instantly,
    // while the background thread safely finishes the .app copy.
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }

    Ok(())
}

/// Copy the running `.app` bundle directly to the user's Desktop. A real
/// bundle (not a symlink) keeps Finder's drop-to-icon open events working
/// and resolves correctly inside LaunchServices.
fn copy_bundle_to_desktop(bundle_dir: &Path, desktop: &Path) -> Result<(), String> {
    let _ = fs::create_dir_all(desktop);

    let dest = desktop.join("CrunchCat.app");
    match fs::symlink_metadata(&dest) {
        Ok(meta) => {
            if meta.file_type().is_dir() {
                let _ = fs::remove_dir_all(&dest);
            } else {
                let _ = fs::remove_file(&dest);
            }
        }
        Err(_) => {}
    }

    let status = Command::new("ditto")
        .arg(bundle_dir)
        .arg(&dest)
        .status()
        .map_err(|e| format!("Failed to run ditto: {e}"))?;
    if !status.success() {
        return Err("ditto failed while copying the app bundle".to_string());
    }

    // Register the copied bundle with LaunchServices so Finder knows it
    // handles the CFBundleDocumentTypes declared in its Info.plist.
    let _ = Command::new(
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
    )
    .arg("-f")
    .arg(&dest)
    .status();

    Ok(())
}
/// Mark the question as answered without creating a shortcut (user chose No).
#[tauri::command]
fn dismiss_shortcut(app: tauri::AppHandle) -> Result<(), String> {
    fs::write(state_file(&app)?, "done").map_err(|e| format!("Failed to save state: {e}"))
}

/// Shared logic: extract archives, compress everything else.
fn process_path(p: &Path) -> Result<String, String> {
    if !p.exists() {
        return Err(format!("Path does not exist: {}", p.display()));
    }

    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ARCHIVE_EXTENSIONS.contains(&ext.as_str()) {
        let output_dir = extract_archive(&p, &ext)?;
        Ok(format!("Extracted to {}", output_dir.display()))
    } else {
        let output_zip = compress_path(&p)?;
        Ok(format!("Compressed to {}", output_zip.display()))
    }
}

/// Process a dropped file/folder from the widget window via IPC.
#[tauri::command]
fn process_dropped_path(path: String) -> Result<String, String> {
    process_path(Path::new(&path))
}

/// Extracts a .zip/.rar/.7z archive into a new directory next to the archive.
fn extract_archive(input: &Path, ext: &str) -> Result<PathBuf, String> {
    let parent = input
        .parent()
        .ok_or_else(|| "Archive has no parent directory".to_string())?;
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("extracted");

    // Standard increment counter for already existing directories (e.g. Song, song (2))
    let mut dest = parent.join(stem);
    let mut counter = 2u32;
    while dest.exists() {
        dest = parent.join(format!("{stem} ({counter})"));
        counter += 1;
    }
    fs::create_dir_all(&dest).map_err(|e| format!("Failed to create output folder: {e}"))?;

    match ext {
        "zip" => {
            let file = File::open(input).map_err(|e| format!("Failed to open zip archive: {e}"))?;
            let mut archive = ZipArchive::new(file)
                .map_err(|e| format!("Failed to read zip archive: {e}"))?;
            archive
                .extract(&dest)
                .map_err(|e| format!("Failed to extract zip archive: {e}"))?;
        }
        "7z" => {
            sevenz_rust::decompress_file(input, &dest)
                .map_err(|e| format!("Failed to extract 7z archive: {e}"))?;
        }
        "rar" => {
            extract_rar(input, &dest)?;
        }
        _ => unreachable!(),
    }

    Ok(dest)
}

/// Extracts a RAR archive with path traversal checks.
fn extract_rar(input: &Path, dest: &Path) -> Result<(), String> {
    let mut archive = unrar::Archive::new(input)
        .open_for_processing()
        .map_err(|e| format!("Failed to open rar archive: {e}"))?;

    while let Some(header) = archive
        .read_header()
        .map_err(|e| format!("Failed to read rar header: {e}"))?
    {
        let entry = header.entry();
        let entry_path = dest.join(&entry.filename);

        // Reject entries that attempt to escape the target directory
        if !entry_path.starts_with(dest) {
            return Err(format!(
                "Unsafe path in rar container: {}",
                entry.filename.display()
            ));
        }

        if entry.is_directory() {
            fs::create_dir_all(&entry_path)
                .map_err(|e| format!("Failed to create rar subfolder: {e}"))?;
            archive = header
                .skip()
                .map_err(|e| format!("Failed to skip rar folder entry: {e}"))?;
        } else {
            if let Some(parent_dir) = entry_path.parent() {
                fs::create_dir_all(parent_dir)
                    .map_err(|e| format!("Failed to create parent paths: {e}"))?;
            }
            archive = header
                .extract_to(&entry_path)
                .map_err(|e| format!("Failed to extract rar file: {e}"))?;
        }
    }

    Ok(())
}

/// Compresses a file/folder into a .zip archive next to it.
fn compress_path(input: &Path) -> Result<PathBuf, String> {
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let file_name = input
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "Invalid file/folder name".to_string())?;

    let stem = match input.extension() {
        Some(_) => input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(file_name)
            .to_string(),
        None => file_name.to_string(),
    };

    let mut output = parent.join(format!("{stem}.zip"));
    let mut counter = 2u32;
    while output.exists() {
        output = parent.join(format!("{stem} ({counter}).zip"));
        counter += 1;
    }

    let file = File::create(&output).map_err(|e| format!("Failed to create zip: {e}"))?;
    let mut zip = ZipWriter::new(file);

    let file_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let dir_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o755);

    if input.is_dir() {
        for entry in WalkDir::new(input)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let relative = path
                .strip_prefix(input)
                .map_err(|e| format!("Failed to build relative path: {e}"))?;
            let rel_name = relative.to_string_lossy().replace('\\', "/");

            if path.is_dir() {
                zip.add_directory(rel_name, dir_options)
                    .map_err(|e| format!("Failed to register zip directory: {e}"))?;
            } else {
                zip.start_file(rel_name, file_options)
                    .map_err(|e| format!("Failed to entry-write zip: {e}"))?;
                let mut src = File::open(path)
                    .map_err(|e| format!("Failed to open file entry: {e}"))?;
                io::copy(&mut src, &mut zip)
                    .map_err(|e| format!("Failed to copy file into zip: {e}"))?;
            }
        }
    } else {
        zip.start_file(file_name.to_string(), file_options)
            .map_err(|e| format!("Failed to entry-write zip: {e}"))?;
        let mut src = File::open(input).map_err(|e| format!("Failed to open file: {e}"))?;
        io::copy(&mut src, &mut zip).map_err(|e| format!("Failed to copy file into zip: {e}"))?;
    }

    zip.finish()
        .map_err(|e| format!("Failed to write zip metadata trailer: {e}"))?;

    Ok(output)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            process_dropped_path,
            shortcut_state,
            create_desktop_shortcut,
            dismiss_shortcut
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let RunEvent::Opened { urls } = event {
                handle_icon_drop(&urls);
            }
        });
}
