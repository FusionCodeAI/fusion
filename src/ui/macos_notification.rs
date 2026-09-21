//! macOS Notification Center Identity Registration.
//!
//! Creates and registers an application bundle identity for the Fusion CLI
//! with macOS LaunchServices so notifications appear in macOS Notification Center
//! with official Fusion branding ("Fusion" app name and icon) instead of "Script Editor".

#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::symlink;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::LazyLock;

#[cfg(target_os = "macos")]
const DEV_APP_DIRECTORY: &str = "notification-identity";
#[cfg(target_os = "macos")]
const DEV_BUNDLE_NAME: &str = "Fusion.app";
#[cfg(target_os = "macos")]
const BUNDLE_IDENTIFIER: &str = "ai.fusion.desktop";
#[cfg(target_os = "macos")]
const APP_DISPLAY_NAME: &str = "Fusion";
#[cfg(target_os = "macos")]
const LAUNCH_SERVICES_REGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

#[cfg(target_os = "macos")]
static ICON_BYTES: &[u8] = include_bytes!("../../apps/fusion-desktop/src-tauri/icons/icon.icns");

#[cfg(target_os = "macos")]
static CONFIGURATION: LazyLock<Result<(), String>> = LazyLock::new(|| {
    if let Err(err) = register_identity_application() {
        tracing::warn!("macOS LaunchServices configure warning: {err}");
    }
    notify_rust::set_application(BUNDLE_IDENTIFIER).map_err(|error| {
        format!("failed configuring macOS notification application {BUNDLE_IDENTIFIER}: {error}")
    })
});

#[cfg(target_os = "macos")]
pub fn configure() -> Result<(), String> {
    CONFIGURATION.clone()
}

#[cfg(not(target_os = "macos"))]
pub fn configure() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn register_identity_application() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed resolving current executable: {error}"))?;

    let base_dir = dirs::home_dir()
        .map(|h| h.join(".fusion"))
        .unwrap_or_else(|| {
            executable
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        });

    let bundle = create_identity_app_bundle(&base_dir, &executable)?;
    let output = Command::new(LAUNCH_SERVICES_REGISTER)
        .arg("-lint")
        .arg("-f")
        .arg(&bundle)
        .output()
        .map_err(|error| format!("failed starting Launch Services registration: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Launch Services registration failed with status {}", output.status)
        } else {
            format!("Launch Services registration failed: {stderr}")
        });
    }

    Ok(bundle)
}

#[cfg(target_os = "macos")]
fn create_identity_app_bundle(base_dir: &Path, executable: &Path) -> Result<PathBuf, String> {
    let bundle = base_dir.join(DEV_APP_DIRECTORY).join(DEV_BUNDLE_NAME);
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");

    fs::create_dir_all(&macos)
        .and_then(|_| fs::create_dir_all(&resources))
        .map_err(|error| format!("failed creating notification identity app bundle: {error}"))?;

    let bundled_executable = macos.join("fusion-app");
    match fs::symlink_metadata(&bundled_executable) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let current_target = fs::read_link(&bundled_executable).map_err(|error| {
                format!("failed reading notification app executable link: {error}")
            })?;
            if current_target != executable {
                let _ = fs::remove_file(&bundled_executable);
                let _ = symlink(executable, &bundled_executable);
            }
        }
        Ok(_) => {
            let _ = fs::remove_file(&bundled_executable);
            let _ = symlink(executable, &bundled_executable);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = symlink(executable, &bundled_executable);
        }
        Err(error) => {
            return Err(format!("failed inspecting notification app executable link: {error}"));
        }
    }

    let icon_path = resources.join("icon.icns");
    if !icon_path.exists() || fs::metadata(&icon_path).map(|m| m.len()).unwrap_or(0) != ICON_BYTES.len() as u64 {
        let _ = fs::write(&icon_path, ICON_BYTES);
    }

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>{APP_DISPLAY_NAME}</string>
  <key>CFBundleExecutable</key>
  <string>fusion-app</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundleIdentifier</key>
  <string>{BUNDLE_IDENTIFIER}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>{APP_DISPLAY_NAME}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>2.0.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
"#
    );

    fs::write(contents.join("Info.plist"), plist_content)
        .map_err(|error| format!("failed writing notification identity metadata: {error}"))?;

    Ok(bundle)
}
