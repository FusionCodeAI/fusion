# Specification: Native macOS Notification Center for Fusion CLI

- **Date**: 2026-09-22
- **Topic**: Replace `osascript` (AppleScript / Script Editor) with genuine native macOS Notification Center alerts matching Fusion Desktop.
- **Status**: Approved

---

## 1. Problem Statement

When the Fusion Rust CLI (`fusion`) dispatches notifications on macOS (e.g. on turn completion, subagent finished, or `/notify test`), it previously rendered and executed an AppleScript command via `/usr/bin/osascript`:
```applescript
display notification "..." with title "Fusion"
```
Because macOS attributes all notifications initiated by `osascript` to Apple's Script Editor (`com.apple.ScriptEditor2`), users see:
- Notification header: **"Script Editor"**
- Notification icon: **Script Editor scroll and quill**
- Script Editor permission dialogs instead of Fusion alerts

The desktop application (`apps/fusion-desktop`) eliminated this by registering an application bundle with Launch Services and using `notify-rust` / `mac-notification-sys` (`NSUserNotification` / `UNUserNotificationCenter`). The CLI must use the same native Notification Center pipeline.

---

## 2. Architecture & Design

### 2.1 Dependencies (`Cargo.toml`)
Add `notify-rust` scoped exclusively to macOS:
```toml
[target.'cfg(target_os = "macos")'.dependencies]
notify-rust = "4.18"
```
This avoids bloating non-macOS targets (Linux, Windows, WASM, Android) while giving the macOS build access to native `mac-notification-sys`.

### 2.2 App Identity Registration (`src/ui/macos_notification.rs`)
macOS Notification Center requires a registered application bundle identifier in Launch Services for notifications to display custom app branding.

1. **Identity Bundle Location**:
   - Primary: `~/.fusion/notification-identity/Fusion.app`
   - Structure:
     ```
     ~/.fusion/notification-identity/Fusion.app/
     └── Contents/
         ├── Info.plist
         ├── MacOS/
         │   └── fusion-app (symlink to current_exe)
         └── Resources/
             └── icon.icns (embedded binary asset)
     ```
2. **Metadata (`Info.plist`)**:
   - `CFBundleIdentifier`: `ai.fusion.desktop`
   - `CFBundleName`: `Fusion`
   - `CFBundleDisplayName`: `Fusion`
   - `CFBundleIconFile`: `icon.icns`
3. **Embedded Icon**:
   - Embed `apps/fusion-desktop/src-tauri/icons/icon.icns` directly in the binary using `include_bytes!`.
   - On initialization, write `icon.icns` into `Resources/` if missing or size differs.
4. **Launch Services Registration**:
   - Execute `/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -lint -f <bundle_path>`.
   - Thread-safe, run-once initialization using `std::sync::OnceLock`.
5. **Notify-Rust Application Binding**:
   - Invoke `notify_rust::set_application("ai.fusion.desktop")`.

### 2.3 Notification Delivery (`src/ui/notify.rs`)
1. **Backend Detection**:
   - On macOS, `NotificationBackend::detect()` returns `NotificationBackend::MacOS`.
   - Label updated from `"macos (osascript)"` to `"macos (native notification center)"`.
2. **Command Dispatch (`send_desktop`)**:
   - On macOS, call `macos_notification::configure()` to guarantee bundle identity registration.
   - Construct and dispatch `notify_rust::Notification`:
     ```rust
     let mut notif = notify_rust::Notification::new();
     notif.summary(&self.title)
          .body(&self.body)
          .auto_icon();
     if self.sound {
         notif.sound_name("default");
     }
     notif.show()?;
     ```
   - Completely remove `osascript` execution from the macOS delivery path.
3. **Slash Command & REPL Feedback (`src/ui/slash.rs`)**:
   - Display `Sent` for desktop notifications on macOS when triggered by `/notify test`.

---

## 3. Data Flow

```
[Agent Turn / User Command / /notify test]
                    │
                    ▼
          Notification::send()
                    │
                    ├──► Terminal OSC (Warp / iTerm / Ghostty)
                    │
                    └──► send_desktop(NotificationBackend::MacOS)
                                │
                                ▼
               macos_notification::configure()
                                │
                  ┌─────────────┴─────────────┐
                  ▼                           ▼
          Bundle Exists?               Register via lsregister
                  │                           │
                  └─────────────┬─────────────┘
                                ▼
                notify_rust::set_application("ai.fusion.desktop")
                                │
                                ▼
                notify_rust::Notification::show()
                                │
                                ▼
             macOS Native Notification Center Alert
             [Fusion Icon | "Fusion" | Title & Body]
```

---

## 4. Error Handling & Fallbacks
- If `lsregister` fails or returns a non-zero exit code (e.g. running in a heavily sandboxed environment), log a diagnostic warning and proceed with `notify-rust` dispatch.
- If `notify_rust::Notification::show()` encounters an error, return `NotificationError::ExecutionFailed`.
- Terminal OSC notifications remain enabled as a complementary channel so terminal emulators (Warp, iTerm2, Kitty, Ghostty) continue receiving tab/activity alerts.

---

## 5. Verification Plan
1. **Compilation Check**: `cargo check --bin fusion` on macOS.
2. **Test Suite**: Run `cargo test --lib ui::notify` to verify tests pass and updated backend names match expectations.
3. **Live Test**: Run `/notify test` or execute notification dispatch in the compiled binary to confirm delivery into macOS Notification Center under "Fusion" with no Script Editor references.
