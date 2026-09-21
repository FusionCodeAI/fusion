# Native macOS Notification Center for Fusion CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `osascript` (AppleScript / Script Editor) with native macOS Notification Center alerts in the Fusion Rust CLI using `notify-rust` and LaunchServices application identity registration matching Fusion Desktop.

**Architecture:** Add `notify-rust` to macOS target dependencies in `Cargo.toml`. Create `src/ui/macos_notification.rs` with an embedded application icon and LaunchServices bundle registration (`~/.fusion/notification-identity/Fusion.app` via `lsregister`). Update `src/ui/notify.rs` so macOS desktop notifications configure the application identity and dispatch directly via `notify_rust::Notification::show()`, completely removing `osascript`. Update CLI status reporting and tests.

**Tech Stack:** Rust (2021 edition), `notify-rust 4.18` (macOS `mac-notification-sys`), macOS LaunchServices (`lsregister`), CoreFoundation / Cocoa `NSUserNotification` / `UNUserNotificationCenter`.

---

### Task 1: Add `notify-rust` to macOS Dependencies in `Cargo.toml`

**Files:**
- Modify: `Cargo.toml:425-460`

- [ ] **Step 1: Edit `Cargo.toml` to add `notify-rust` under macOS target dependencies**

Add target configuration for macOS dependencies:
```toml
[target.'cfg(target_os = "macos")'.dependencies]
notify-rust = "4.18"
```

- [ ] **Step 2: Verify `Cargo.toml` resolves and compiles**

Run: `cargo check --bin fusion`
Expected: PASS with `notify-rust` and `mac-notification-sys` compiled into target dependencies.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add notify-rust for macOS targets in Fusion CLI"
```

---

### Task 2: Create `src/ui/macos_notification.rs` for App Identity Registration

**Files:**
- Create: `src/ui/macos_notification.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Implement `src/ui/macos_notification.rs`**

```rust
#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::symlink;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::OnceLock;

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
static CONFIGURATION: OnceLock<Result<(), String>> = OnceLock::new();

#[cfg(target_os = "macos")]
pub fn configure() -> Result<(), String> {
    CONFIGURATION
        .get_or_init(|| {
            if let Err(err) = register_identity_application() {
                eprintln!("[notification] macOS LaunchServices configure warning: {err}");
            }
            notify_rust::set_application(BUNDLE_IDENTIFIER).map_err(|error| {
                format!("failed configuring macOS notification application {BUNDLE_IDENTIFIER}: {error}")
            })
        })
        .clone()
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
```

- [ ] **Step 2: Expose `macos_notification` in `src/ui/mod.rs`**

Add `pub mod macos_notification;` in `src/ui/mod.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check --bin fusion`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/ui/macos_notification.rs src/ui/mod.rs
git commit -m "feat(ui): add macOS notification identity registration using LaunchServices"
```

---

### Task 3: Update `src/ui/notify.rs` for Native Notification Center

**Files:**
- Modify: `src/ui/notify.rs`

- [ ] **Step 1: Write unit tests verifying updated backend detection and naming**

In `src/ui/notify.rs` tests module:
```rust
#[test]
fn test_macos_backend_name_is_native() {
    assert_eq!(NotificationBackend::MacOS.name(), "macos (notification center)");
}
```

- [ ] **Step 2: Update `NotificationBackend` in `src/ui/notify.rs`**

1. In `NotificationBackend::detect()`:
```rust
#[cfg(target_os = "macos")]
{
    return Self::MacOS;
}
```
2. In `NotificationBackend::name()`:
Change `Self::MacOS => "macos (osascript)",` to `Self::MacOS => "macos (notification center)",`.

- [ ] **Step 3: Update `send_desktop` in `src/ui/notify.rs`**

On macOS:
```rust
#[cfg(target_os = "macos")]
{
    if backend == NotificationBackend::MacOS || backend == NotificationBackend::Auto {
        if let Err(err) = crate::ui::macos_notification::configure() {
            tracing::warn!("Failed configuring macOS notification identity: {err}");
        }

        let mut notif = notify_rust::Notification::new();
        notif.summary(&self.title)
            .body(&self.body)
            .auto_icon();

        if let Some(sub) = &self.subtitle {
            if !sub.is_empty() {
                notif.subtitle(sub);
            }
        }

        if self.sound {
            notif.sound_name("default");
        }

        return notif
            .show()
            .map(|_| ())
            .map_err(|e| NotificationError::ExecutionFailed(format!("notify_rust error: {e}")));
    }
}
```
Remove `render_macos_command` that invoked `osascript`.

- [ ] **Step 4: Run tests**

Run: `cargo test --lib ui::notify`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/ui/notify.rs
git commit -m "feat(ui): dispatch macOS notifications directly to Notification Center via notify-rust"
```

---

### Task 4: Update `src/ui/slash.rs` CLI Test Output

**Files:**
- Modify: `src/ui/slash.rs:1690-1725`

- [ ] **Step 1: Enable Desktop OS status printing on macOS**

In `src/ui/slash.rs`:
Remove `#[cfg(not(target_os = "macos"))]` gating around the `Desktop OS: Sent / Not Sent` output in `handle_notify("test")` so macOS users see delivery status just like other platforms.

- [ ] **Step 2: Run `cargo check --bin fusion`**

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/ui/slash.rs
git commit -m "feat(ui): display desktop notification delivery status on macOS in /notify test"
```

---

### Task 5: End-to-End Verification

**Files:**
- Tests: `cargo test --lib ui::notify`
- Integration: `/notify test` via CLI test command

- [ ] **Step 1: Run full test suite for notifications**

Run: `cargo test --lib ui::notify`
Expected: All notification tests PASS.

- [ ] **Step 2: Test native notification delivery with the compiled binary**

Run: `cargo run --bin fusion -- notify test`
Expected:
1. Console shows `Desktop OS: Sent`.
2. A genuine macOS Notification Center banner appears with the title `Fusion`, icon from `icon.icns`, and the test message.
3. No Script Editor app or icon is involved.

- [ ] **Step 3: Commit any adjustments**

```bash
git commit --allow-empty -m "chore: complete verification for native macOS Notification Center"
```
