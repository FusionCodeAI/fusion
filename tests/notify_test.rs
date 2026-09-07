//! Targeted integration tests for terminal notification protocols, tmux DCS passthrough,
//! Zellij BEL signaling, rich OSC 99 Kitty desktop notification protocol, and explicit event triggers.

use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

use fusion::ui::notify::{
    format_multiplexer_terminal_sequence, format_osc99_notification, format_osc99_with_trigger,
    is_inside_tmux, is_inside_zellij, wrap_tmux_passthrough, Notification, NotificationPriority,
    NotificationTrigger, NotificationUrgency, TERMINAL_BELL,
};

#[test]
fn test_tmux_passthrough_envelope_wrapping() {
    // Reference test vector from OMP packages/tui/test/notifications.test.ts:
    // wrapTmuxPassthrough("\x1b]99;;Hello\x1b\\") -> "\x1bPtmux;\x1b\x1b]99;;Hello\x1b\x1b\\\x1b\\"
    let payload = "\x1b]99;;Hello\x1b\\";
    let wrapped = wrap_tmux_passthrough(payload);
    assert_eq!(wrapped, "\x1bPtmux;\x1b\x1b]99;;Hello\x1b\x1b\\\x1b\\");

    // Double ESC wrapping for OSC 9 with BEL
    let osc9_payload = "\x1b]9;Turn Complete: 42s\x07";
    let wrapped_osc9 = wrap_tmux_passthrough(osc9_payload);
    assert_eq!(
        wrapped_osc9,
        "\x1bPtmux;\x1b\x1b]9;Turn Complete: 42s\x07\x1b\\"
    );
}

#[test]
fn test_multiplexer_detection_flags() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Save original env
    let orig_tmux = std::env::var("TMUX").ok();
    let orig_zellij = std::env::var("ZELLIJ").ok();

    // Outside multiplexers
    std::env::remove_var("TMUX");
    std::env::remove_var("ZELLIJ");
    assert!(!is_inside_tmux());
    assert!(!is_inside_zellij());

    // Inside tmux
    std::env::set_var("TMUX", "/tmp/tmux-501/default,4281,0");
    assert!(is_inside_tmux());
    std::env::remove_var("TMUX");
    assert!(!is_inside_tmux());

    // Inside Zellij
    std::env::set_var("ZELLIJ", "0");
    assert!(is_inside_zellij());
    std::env::remove_var("ZELLIJ");
    assert!(!is_inside_zellij());

    // Restore env
    if let Some(val) = orig_tmux {
        std::env::set_var("TMUX", val);
    }
    if let Some(val) = orig_zellij {
        std::env::set_var("ZELLIJ", val);
    }
}

#[test]
fn test_multiplexer_sequence_formatting() {
    let orig_tmux = std::env::var("TMUX").ok();
    let _guard = ENV_LOCK.lock().unwrap();
    let orig_zellij = std::env::var("ZELLIJ").ok();

    let osc_seq = "\x1b]99;i=1:d=0;Session\x1b\\\x1b]99;i=1:p=body;Complete\x1b\\";

    // 1. Outside multiplexer: sequence is untouched
    std::env::remove_var("TMUX");
    std::env::remove_var("ZELLIJ");
    assert_eq!(format_multiplexer_terminal_sequence(osc_seq), osc_seq);

    // 2. Inside tmux: wrapped in DCS passthrough followed by BEL (\x07)
    std::env::set_var("TMUX", "/tmp/tmux-1000/default,1234,0");
    let tmux_formatted = format_multiplexer_terminal_sequence(osc_seq);
    assert!(tmux_formatted.starts_with("\x1bPtmux;\x1b\x1b]99;"));
    assert!(tmux_formatted.ends_with("\x1b\\\x07"));
    std::env::remove_var("TMUX");

    // 3. Inside Zellij: appended BEL (\x07) without DCS wrapping
    std::env::set_var("ZELLIJ", "session-1");
    let zellij_formatted = format_multiplexer_terminal_sequence(osc_seq);
    assert_eq!(zellij_formatted, format!("{osc_seq}\x07"));
    assert!(!zellij_formatted.contains("\x1bPtmux;"));
    std::env::remove_var("ZELLIJ");

    // 4. Pure Bell (\x07) is never DCS-wrapped even under tmux (already flags monitor-bell)
    std::env::set_var("TMUX", "/tmp/tmux-1000/default,1234,0");
    assert_eq!(
        format_multiplexer_terminal_sequence(TERMINAL_BELL),
        TERMINAL_BELL
    );
    std::env::remove_var("TMUX");

    // Restore
    if let Some(val) = orig_tmux {
        std::env::set_var("TMUX", val);
    }
    if let Some(val) = orig_zellij {
        std::env::set_var("ZELLIJ", val);
    }
}

#[test]
fn test_rich_osc99_kitty_protocol_title_and_body() {
    let notif = Notification::new("Build Succeeded", "All 42 targets compiled clean")
        .category("build-1")
        .app_name("Fusion")
        .priority(NotificationPriority::Success)
        .trigger(NotificationTrigger::Completion)
        .timeout_ms(4000);

    let osc99 = notif.render_osc99();

    // Verify chunk 1 contains metadata, id, application name, trigger, and title
    assert!(osc99.contains("\x1b]99;i=build-1:"));
    assert!(osc99.contains("a=focus"));
    assert!(osc99.contains("u=1")); // normal urgency
    assert!(osc99.contains("w=4000"));
    assert!(osc99.contains("d=0;Build Succeeded\x1b\\"));

    // Verify chunk 2 contains body payload with p=body and matching id
    assert!(osc99.contains("\x1b]99;i=build-1:p=body;All 42 targets compiled clean\x1b\\"));
}

#[test]
fn test_rich_osc99_unsafe_characters_base64_encoding() {
    // Unsafe C0 control characters such as newlines must be base64-encoded with e=1
    let notif = Notification::new("Multi\nLine\nTitle", "Line A\nLine B")
        .category("unsafe-test");

    let osc99 = notif.render_osc99();
    assert!(osc99.contains(":e=1;"));
    // Title encoded
    assert!(!osc99.contains("Multi\nLine\nTitle"));
    // Body encoded
    assert!(!osc99.contains("Line A\nLine B"));
}

#[test]
fn test_format_osc99_notification_helpers() {
    let simple_osc = format_osc99_notification("Quick Title", "Quick Body");
    assert!(simple_osc.contains("Quick Title"));
    assert!(simple_osc.contains("p=body;Quick Body"));

    let trigger_osc = format_osc99_with_trigger("Task", "Done", NotificationTrigger::Completion);
    assert!(trigger_osc.contains("Task"));
    assert!(trigger_osc.contains("p=body;Done"));
}

#[test]
fn test_explicit_triggers_completion_error_ask() {
    // 1. Completion trigger
    let comp = Notification::completion("Session", "Agent finished task");
    assert_eq!(comp.get_trigger(), Some(NotificationTrigger::Completion));
    assert_eq!(comp.get_priority(), NotificationPriority::Success);
    assert_eq!(comp.urgency, NotificationUrgency::Normal);

    // 2. Error trigger
    let err = Notification::error("Crash", "Subprocess returned 137");
    assert_eq!(err.get_trigger(), Some(NotificationTrigger::Error));
    assert_eq!(err.get_priority(), NotificationPriority::Error);
    assert_eq!(err.urgency, NotificationUrgency::Critical);
    assert!(err.sound);

    // 3. Ask trigger
    let ask = Notification::ask("Permission Needed", "Allow running rm -rf target/?");
    assert_eq!(ask.get_trigger(), Some(NotificationTrigger::Ask));
    assert_eq!(ask.get_priority(), NotificationPriority::Warning);
    assert!(ask.sound);
}

#[test]
fn test_send_terminal_osc_writer_with_multiplexer() {
    let _guard = ENV_LOCK.lock().unwrap();
    let orig_tmux = std::env::var("TMUX").ok();
    let orig_zellij = std::env::var("ZELLIJ").ok();

    std::env::set_var("TMUX", "/tmp/tmux-test");
    let notif = Notification::completion("Turn", "Completed");

    let mut buf = Vec::new();
    let result = notif.send_terminal_osc(&mut buf);
    assert!(result.is_ok());

    let written = String::from_utf8_lossy(&buf);
    // Under tmux, written sequence must start with tmux DCS and end with BEL
    assert!(written.starts_with("\x1bPtmux;"));
    assert!(written.ends_with("\x07"));

    std::env::remove_var("TMUX");

    // Restore
    if let Some(val) = orig_tmux {
        std::env::set_var("TMUX", val);
    }
    if let Some(val) = orig_zellij {
        std::env::set_var("ZELLIJ", val);
    }
}
