//! Clean FX-style usage and quota card renderer for Fusion backend metrics.
//!
//! Provides ANSI-styled terminal cards for backend account telemetry (`/v1/usage`):
//! - Plan name and account identification.
//! - Current monthly spend vs limit with visual progress meters.
//! - Remaining balance indicator.
//! - Monthly token consumption and prompt cache hit rate statistics.
//! - Financial savings from prefix caching.
//! - Per-model prefix caching savings breakdown.
//! - Graceful error presentation when backend connectivity fails.

use std::collections::HashMap;

use crate::provider::usage::BackendUsageReport;
use crate::ui::table::visible_width;

// ============================================================================
// Constants & Styling
// ============================================================================

/// Default card width in terminal columns.
pub const DEFAULT_CARD_WIDTH: usize = 64;

/// Minimum allowable card width in terminal columns.
pub const MIN_CARD_WIDTH: usize = 48;

// Progress meter block characters
const METER_FULL: &str = "█";
const METER_SEVEN_EIGHTHS: &str = "▉";
const METER_THREE_QUARTERS: &str = "▊";
const METER_FIVE_EIGHTHS: &str = "▋";
const METER_HALF: &str = "▌";
const METER_THREE_EIGHTHS: &str = "▍";
const METER_ONE_QUARTER: &str = "▎";
const METER_ONE_EIGHTH: &str = "▏";
const METER_EMPTY: &str = "░";
// ANSI escape sequences
#[allow(dead_code)]
const ANSI_RESET: &str = "\x1b[0m";
#[allow(dead_code)]
const ANSI_BOLD: &str = "\x1b[1m";
#[allow(dead_code)]
const ANSI_DIM: &str = "\x1b[2m";
#[allow(dead_code)]
const ANSI_CYAN: &str = "\x1b[36m";
#[allow(dead_code)]
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
#[allow(dead_code)]
const ANSI_GREEN: &str = "\x1b[32m";
#[allow(dead_code)]
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
#[allow(dead_code)]
const ANSI_YELLOW: &str = "\x1b[33m";
#[allow(dead_code)]
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
#[allow(dead_code)]
const ANSI_RED: &str = "\x1b[31m";
#[allow(dead_code)]
const ANSI_BOLD_RED: &str = "\x1b[1;31m";
#[allow(dead_code)]
const ANSI_GRAY: &str = "\x1b[90m";
#[allow(dead_code)]
const ANSI_WHITE: &str = "\x1b[37m";
#[allow(dead_code)]
const ANSI_BOLD_WHITE: &str = "\x1b[1;37m";
// ============================================================================
// Formatting Helpers
// ============================================================================

/// Formats a token count into a compact string with suffix (e.g. `1.45M`, `125.0k`, `500`).
pub fn format_tokens_compact(tokens: u64) -> String {
    if tokens >= 1_000_000_000 {
        let val = tokens as f64 / 1_000_000_000.0;
        format!("{:.2}B", val)
    } else if tokens >= 1_000_000 {
        let val = tokens as f64 / 1_000_000.0;
        format!("{:.2}M", val)
    } else if tokens >= 10_000 {
        let val = tokens as f64 / 1_000.0;
        format!("{:.1}k", val)
    } else if tokens >= 1_000 {
        let val = tokens as f64 / 1_000.0;
        format!("{:.1}k", val)
    } else {
        tokens.to_string()
    }
}

/// Formats an integer with thousands comma separators (e.g. `1,450,200`).
pub fn format_number_commas(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

/// Formats a USD monetary amount (e.g. `$1.24`, `$50.00`, `-$0.50`).
pub fn format_currency(amount: f64) -> String {
    if amount.abs() < 1e-9 {
        "$0.00".to_string()
    } else if amount < 0.0 {
        format!("-${:.2}", amount.abs())
    } else if amount < 0.01 && amount > 0.0 {
        format!("${:.4}", amount)
    } else {
        format!("${:.2}", amount)
    }
}

/// Formats cache savings with a leading plus sign (e.g. `+$0.42` or `$0.00`).
pub fn format_savings(amount: f64) -> String {
    if amount > 1e-9 {
        format!("+{}", format_currency(amount))
    } else {
        format_currency(amount)
    }
}

/// Builds a horizontal Unicode meter bar representing a fraction between 0.0 and 1.0.
pub fn render_meter_bar(ratio: f64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let clamped = ratio.clamp(0.0, 1.0);
    let full_units = (clamped * width as f64).floor() as usize;
    let remainder = (clamped * width as f64) - full_units as f64;

    let mut bar = String::with_capacity(width * 4);

    for _ in 0..full_units {
        bar.push_str(METER_FULL);
    }

    if full_units < width {
        let partial = if remainder >= 0.875 {
            METER_SEVEN_EIGHTHS
        } else if remainder >= 0.75 {
            METER_THREE_QUARTERS
        } else if remainder >= 0.625 {
            METER_FIVE_EIGHTHS
        } else if remainder >= 0.5 {
            METER_HALF
        } else if remainder >= 0.375 {
            METER_THREE_EIGHTHS
        } else if remainder >= 0.25 {
            METER_ONE_QUARTER
        } else if remainder >= 0.125 {
            METER_ONE_EIGHTH
        } else {
            METER_EMPTY
        };

        bar.push_str(partial);

        let empty_units = width.saturating_sub(full_units + 1);
        for _ in 0..empty_units {
            bar.push_str(METER_EMPTY);
        }
    }

    bar
}

/// Wraps text into multiple lines bounded by `max_width`.
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            lines.push(String::new());
            continue;
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let mut cur = String::new();
        for word in words {
            if cur.is_empty() {
                cur.push_str(word);
            } else if cur.len() + 1 + word.len() <= max_width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                lines.push(cur);
                cur = word.to_string();
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    lines
}

// ============================================================================
// Primary Renderers
// ============================================================================

/// Renders a comprehensive backend usage report into a clean FX-styled ANSI card.
pub fn render_backend_usage_fx(report: &BackendUsageReport) -> String {
    let target_width = DEFAULT_CARD_WIDTH.max(MIN_CARD_WIDTH);
    let inner_width = target_width.saturating_sub(2);

    let mut out = String::new();

    // Helper closure for boxing a line
    let box_line = |content: &str| -> String {
        let vis_len = visible_width(content);
        let pad = inner_width.saturating_sub(vis_len);
        format!("│{}{}│\n", content, " ".repeat(pad))
    };

    // Helper closure for standard divider
    let div_line = || -> String { format!("├{}┤\n", "─".repeat(inner_width)) };

    // Helper closure for labeled divider
    // 1. Clean top border without title
    out.push_str(&format!("╭{}╮\n", "─".repeat(inner_width)));

    // 2. Plan and Account Info Line
    let plan_name_trimmed = report.plan_name.trim();
    let plan_str = if plan_name_trimmed.is_empty() {
        "Default".to_string()
    } else {
        plan_name_trimmed.to_string()
    };

    let plan_display = if report.is_payg
        && !plan_str.to_lowercase().contains("payg")
        && !plan_str.to_lowercase().contains("pay as you go")
    {
        format!("{} [PAYG]", plan_str)
    } else {
        plan_str
    };

    let plan_tag = format!(
        "  {}Plan:{} {}{}{}",
        ANSI_GRAY, ANSI_RESET, ANSI_BOLD_WHITE, plan_display, ANSI_RESET
    );

    let line1 = if let Some(email) = &report.user_email {
        let email_trimmed = email.trim();
        if !email_trimmed.is_empty() {
            let email_tag = format!(
                "{}Account:{} {}{}{}",
                ANSI_GRAY, ANSI_RESET, ANSI_CYAN, email_trimmed, ANSI_RESET
            );
            let p_vis = visible_width(&plan_tag);
            let e_vis = visible_width(&email_tag);
            let spacing = inner_width.saturating_sub(p_vis + e_vis + 2);
            format!("{}{}{}{}", plan_tag, " ".repeat(spacing), email_tag, "  ")
        } else {
            plan_tag
        }
    } else {
        plan_tag
    };
    out.push_str(&box_line(&line1));

    out.push_str(&div_line());

    // 3. Usage vs Monthly Limit & Progress Meter
    let is_capped = report.monthly_limit_usd > 0.0;
    let pct = if is_capped {
        report.usage_percentage()
    } else {
        0.0
    };

    let usage_str = if is_capped {
        format!(
            "${:.2} / ${:.2} ({:.1}%)",
            report.used_usd, report.monthly_limit_usd, pct
        )
    } else if report.is_payg {
        format!("${:.2} (Pay As You Go)", report.used_usd)
    } else {
        format!("${:.2}", report.used_usd)
    };

    let usage_row = format!(
        "  {}Usage:             {}{}{}{}",
        ANSI_GRAY, ANSI_RESET, ANSI_BOLD_WHITE, usage_str, ANSI_RESET
    );
    out.push_str(&box_line(&usage_row));

    // Progress Bar
    if is_capped {
        let bar_width = 28;
        let ratio = (report.used_usd / report.monthly_limit_usd).clamp(0.0, 1.0);
        let bar = render_meter_bar(ratio, bar_width);

        let (bar_color, badge) = if pct >= 100.0 {
            (
                ANSI_BOLD_RED,
                format!(
                    " {}{}[LIMIT EXCEEDED]{}",
                    ANSI_BOLD_RED, ANSI_BOLD, ANSI_RESET
                ),
            )
        } else if pct >= 80.0 {
            (
                ANSI_BOLD_YELLOW,
                format!(
                    " {}{}[NEAR LIMIT]{}",
                    ANSI_BOLD_YELLOW, ANSI_BOLD, ANSI_RESET
                ),
            )
        } else {
            (ANSI_BOLD_CYAN, String::new())
        };

        let meter_str = format!(
            "  {}Quota:             {}[{}{}{}]{} {:>5.1}%{}",
            ANSI_GRAY, ANSI_RESET, bar_color, bar, ANSI_RESET, ANSI_DIM, pct, badge
        );
        out.push_str(&box_line(&meter_str));
    } else {
        let meter_str = format!(
            "  {}Quota:             {}[{}No monthly spending limit{}]",
            ANSI_GRAY, ANSI_RESET, ANSI_DIM, ANSI_RESET
        );
        out.push_str(&box_line(&meter_str));
    }

    // Remaining Balance
    let remaining_row = if is_capped {
        let rem_color = if report.remaining_usd > 0.0 {
            ANSI_BOLD_GREEN
        } else if report.remaining_usd.abs() < 1e-9 {
            ANSI_BOLD_YELLOW
        } else {
            ANSI_BOLD_RED
        };
        format!(
            "  {}Remaining Balance: {}{}{}{}",
            ANSI_GRAY,
            ANSI_RESET,
            rem_color,
            format_currency(report.remaining_usd),
            ANSI_RESET
        )
    } else {
        format!(
            "  {}Remaining Balance: {}{}Unlimited (Pay As You Go){}",
            ANSI_GRAY, ANSI_RESET, ANSI_BOLD_GREEN, ANSI_RESET
        )
    };
    out.push_str(&box_line(&remaining_row));

    out.push_str(&div_line());

    // 4. Monthly Token Activity & Cache Efficiency
    let tokens_compact = format_tokens_compact(report.used_tokens_this_month);
    let tokens_detail = if report.used_tokens_this_month >= 10_000 {
        format!(
            "{} tokens {}({}){}",
            tokens_compact,
            ANSI_DIM,
            format_number_commas(report.used_tokens_this_month),
            ANSI_RESET
        )
    } else {
        format!("{} tokens", report.used_tokens_this_month)
    };

    let tokens_row = format!(
        "  {}Tokens This Month: {}{}{}{}",
        ANSI_GRAY, ANSI_RESET, ANSI_BOLD_WHITE, tokens_detail, ANSI_RESET
    );
    out.push_str(&box_line(&tokens_row));

    // Cache Hit Rate Line
    let cached_tokens_str = format_tokens_compact(report.cached_tokens_this_month);
    let hit_rate_detail = if report.cache_hit_count_this_month > 0 {
        format!(
            "{:.1}% {}({} cached tokens, {} hits){}",
            report.prompt_cache_hit_rate_pct,
            ANSI_DIM,
            cached_tokens_str,
            format_number_commas(report.cache_hit_count_this_month),
            ANSI_RESET
        )
    } else {
        format!(
            "{:.1}% {}({} cached tokens){}",
            report.prompt_cache_hit_rate_pct, ANSI_DIM, cached_tokens_str, ANSI_RESET
        )
    };

    let hit_rate_row = format!(
        "  {}Cache Hit Rate:    {}{}{}{}",
        ANSI_GRAY, ANSI_RESET, ANSI_BOLD_CYAN, hit_rate_detail, ANSI_RESET
    );
    out.push_str(&box_line(&hit_rate_row));

    // Cache Savings Line
    let savings_str = format!(
        "{} saved via prefix caching",
        format_savings(report.cache_savings_usd_this_month)
    );
    let savings_row = format!(
        "  {}Cache Savings:     {}{}{}{}",
        ANSI_GRAY, ANSI_RESET, ANSI_BOLD_GREEN, savings_str, ANSI_RESET
    );
    out.push_str(&box_line(&savings_row));

    // 6. Bottom border
    out.push_str(&format!("╰{}╯\n", "─".repeat(inner_width)));

    out
}

/// Renders a graceful error card when backend usage retrieval fails.
pub fn render_backend_usage_error(err_msg: &str) -> String {
    let target_width = DEFAULT_CARD_WIDTH.max(MIN_CARD_WIDTH);
    let inner_width = target_width.saturating_sub(2);

    let mut out = String::new();

    let box_line = |content: &str| -> String {
        let vis_len = visible_width(content);
        let pad = inner_width.saturating_sub(vis_len);
        format!("│{}{}│\n", content, " ".repeat(pad))
    };

    // Top border
    let title = "◈ Fusion Usage Error";
    let title_styled = format!(" {}{}{}{} ", ANSI_BOLD_RED, ANSI_BOLD, title, ANSI_RESET);
    let title_vis_len = visible_width(title) + 2;
    let right_border_len = inner_width.saturating_sub(title_vis_len + 1);
    out.push_str(&format!(
        "╭─{}{}{}\n",
        title_styled,
        "─".repeat(right_border_len),
        "╮"
    ));

    out.push_str(&box_line(""));
    out.push_str(&box_line(&format!(
        "  {}⚠ Failed to retrieve backend usage{}",
        ANSI_BOLD_YELLOW, ANSI_RESET
    )));
    out.push_str(&box_line(""));

    out.push_str(&box_line(&format!("  {}Details:{}", ANSI_GRAY, ANSI_RESET)));
    let wrapped = wrap_text(err_msg, inner_width.saturating_sub(6));
    if wrapped.is_empty() {
        out.push_str(&box_line(&format!(
            "    {}Unknown error occurred.{}",
            ANSI_RED, ANSI_RESET
        )));
    } else {
        for line in wrapped {
            out.push_str(&box_line(&format!(
                "    {}{}{}",
                ANSI_RED, line, ANSI_RESET
            )));
        }
    }

    out.push_str(&box_line(""));
    let tip = "Tip: Verify FUSION_API_KEY is set or check network connection.";
    for line in wrap_text(tip, inner_width.saturating_sub(4)) {
        out.push_str(&box_line(&format!("  {}{}{}", ANSI_DIM, line, ANSI_RESET)));
    }
    out.push_str(&box_line(""));

    // Bottom border
    out.push_str(&format!("╰{}╯\n", "─".repeat(inner_width)));

    out
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::table::strip_ansi;

    #[test]
    fn test_format_tokens_compact() {
        assert_eq!(format_tokens_compact(0), "0");
        assert_eq!(format_tokens_compact(500), "500");
        assert_eq!(format_tokens_compact(1_200), "1.2k");
        assert_eq!(format_tokens_compact(25_000), "25.0k");
        assert_eq!(format_tokens_compact(1_450_000), "1.45M");
        assert_eq!(format_tokens_compact(2_500_000_000), "2.50B");
    }

    #[test]
    fn test_format_number_commas() {
        assert_eq!(format_number_commas(0), "0");
        assert_eq!(format_number_commas(999), "999");
        assert_eq!(format_number_commas(1000), "1,000");
        assert_eq!(format_number_commas(1_450_200), "1,450,200");
    }

    #[test]
    fn test_format_currency_and_savings() {
        assert_eq!(format_currency(0.0), "$0.00");
        assert_eq!(format_currency(1.24), "$1.24");
        assert_eq!(format_currency(50.0), "$50.00");
        assert_eq!(format_currency(-0.5), "-$0.50");

        assert_eq!(format_savings(0.0), "$0.00");
        assert_eq!(format_savings(0.42), "+$0.42");
    }

    #[test]
    fn test_render_meter_bar() {
        let empty = render_meter_bar(0.0, 10);
        assert_eq!(empty.chars().count(), 10);

        let half = render_meter_bar(0.5, 10);
        assert_eq!(half.chars().count(), 10);

        let full = render_meter_bar(1.0, 10);
        assert_eq!(full.chars().count(), 10);
        assert_eq!(full, "██████████");
    }

    #[test]
    fn test_render_backend_usage_fx_full() {
        let mut models = HashMap::new();
        models.insert(
            "claude-3-7-sonnet-20250219".to_string(),
            crate::provider::usage::ModelCacheSavings {
                cached_tokens: 350_000,
                savings_usd: 0.35,
            },
        );
        models.insert(
            "claude-3-5-haiku-20241022".to_string(),
            crate::provider::usage::ModelCacheSavings {
                cached_tokens: 70_000,
                savings_usd: 0.07,
            },
        );
        let report = BackendUsageReport {
            user_email: Some("dev@example.com".to_string()),
            plan_name: "Pro".to_string(),
            used_usd: 1.24,
            monthly_limit_usd: 50.0,
            remaining_usd: 48.76,
            used_tokens_this_month: 1_450_000,
            cached_tokens_this_month: 615_000,
            prompt_tokens_this_month: 1_100_000,
            prompt_cache_hit_rate_pct: 42.5,
            cache_hit_count_this_month: 142,
            cache_savings_usd_this_month: 0.42,
            cache_savings_by_model: models,
            is_payg: false,
        };

        let rendered = render_backend_usage_fx(&report);
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("Plan: Pro"));
        assert!(plain.contains("Account: dev@example.com"));
        assert!(plain.contains("$1.24 / $50.00 (2.5%)"));
        assert!(plain.contains("Remaining Balance: $48.76"));
        assert!(plain.contains("1.45M tokens"));
        assert!(plain.contains("42.5%"));
        assert!(plain.contains("+$0.42 saved via prefix caching"));

        // Verify each line has consistent visible width
        for line in plain.lines() {
            let vis = visible_width(line);
            assert_eq!(
                vis, DEFAULT_CARD_WIDTH,
                "Line length mismatch: '{}' (vis={})",
                line, vis
            );
        }
    }

    #[test]
    fn test_render_backend_usage_fx_payg_no_limit() {
        let report = BackendUsageReport {
            user_email: None,
            plan_name: "Pay As You Go".to_string(),
            used_usd: 12.50,
            monthly_limit_usd: 0.0,
            remaining_usd: 0.0,
            used_tokens_this_month: 500_000,
            cached_tokens_this_month: 100_000,
            prompt_tokens_this_month: 400_000,
            prompt_cache_hit_rate_pct: 20.0,
            cache_hit_count_this_month: 30,
            cache_savings_usd_this_month: 0.15,
            cache_savings_by_model: HashMap::new(),
            is_payg: true,
        };

        let rendered = render_backend_usage_fx(&report);
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("Plan: Pay As You Go"));
        assert!(plain.contains("Usage:             $12.50 (Pay As You Go)"));
        assert!(plain.contains("No monthly spending limit"));
        assert!(plain.contains("Unlimited (Pay As You Go)"));
        assert!(!plain.contains("Prefix Cache Savings by Model"));

        for line in plain.lines() {
            let vis = visible_width(line);
            assert_eq!(
                vis, DEFAULT_CARD_WIDTH,
                "Line length mismatch: '{}' (vis={})",
                line, vis
            );
        }
    }

    #[test]
    fn test_render_backend_usage_error() {
        let err = "Failed to connect to Fusion backend: connection refused (os error 61)";
        let rendered = render_backend_usage_error(err);
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("Fusion Usage Error"));
        assert!(plain.contains("Failed to retrieve backend usage"));
        assert!(plain.contains("connection refused"));
        assert!(plain.contains("Tip: Verify FUSION_API_KEY"));

        for line in plain.lines() {
            let vis = visible_width(line);
            assert_eq!(
                vis, DEFAULT_CARD_WIDTH,
                "Line length mismatch: '{}' (vis={})",
                line, vis
            );
        }
    }
    #[test]
    fn test_render_backend_usage_near_and_over_limit() {
        let mut near_report = BackendUsageReport::default();
        near_report.plan_name = "Pro".to_string();
        near_report.used_usd = 42.0;
        near_report.monthly_limit_usd = 50.0;
        near_report.remaining_usd = 8.0;

        let rendered_near = render_backend_usage_fx(&near_report);
        let plain_near = strip_ansi(&rendered_near);
        assert!(plain_near.contains("[NEAR LIMIT]"));
        assert!(plain_near.contains("$42.00 / $50.00 (84.0%)"));

        let mut over_report = BackendUsageReport::default();
        over_report.plan_name = "Pro".to_string();
        over_report.used_usd = 55.0;
        over_report.monthly_limit_usd = 50.0;
        over_report.remaining_usd = -5.0;

        let rendered_over = render_backend_usage_fx(&over_report);
        let plain_over = strip_ansi(&rendered_over);
        assert!(plain_over.contains("[LIMIT EXCEEDED]"));
        assert!(plain_over.contains("Remaining Balance: -$5.00"));
    }
}
