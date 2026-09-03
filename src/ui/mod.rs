pub mod inline;
pub mod markdown;
pub mod prompt;
pub mod repl;
pub mod spinner;
pub mod slash;
pub mod theme;
pub mod keys;
pub mod sound;
pub mod model_picker;
pub mod table;
pub mod termux;
pub mod status;
pub mod budget;
pub mod diff_view;
pub mod voice;
pub mod notify;
pub mod file_picker;
pub mod multicursor;
pub mod quick;
pub mod side_by_side;
pub mod colors;
pub mod title;
pub mod doc_render;
pub mod agent_tree;
pub mod bench_cmd;
pub mod bench_runner;
pub mod context_view;
pub mod prompt_match;
pub mod context_warning;
pub mod progress_tree;
pub mod mouse;
pub mod print_css;
pub mod stats_card;
pub mod keymap_config;
pub mod banner;
pub mod rate_limit_banner;
// Re-exports for convenient top-level access
pub use inline::{
    calculate_text_height, render_card, render_card_themed, render_critique_card,
    render_critique_card_themed, render_status_bar, render_status_bar_themed, InlineTerminal,
    StatusInfo,
};
pub use markdown::{print_markdown, render_inline, render_line, render_markdown, MarkdownRenderer};
pub use prompt::{Prompt, PromptResult, RawModeGuard};
pub use repl::{
    format_duration_compact as format_repl_duration_compact,
    format_model_label, format_thinking_status, format_tokens_compact as format_repl_tokens_compact,
    format_tool_tree, format_turn_summary, handle_command, parse_tool_info, print_banner,
    print_help, render_tool_tree, render_tool_tree_to, run_repl, run_turn_ui, ToolCallItem,
};
pub use spinner::{format_duration, Spinner, SpinnerHandle};
pub use slash::{
    execute_slash_command, get_command_palette, handle_slash_command, print_command_palette,
    render_command_palette, CommandCategory, CommandDescriptor, CommandResult, ConfigCommand,
    ExportFormat, PromptCommand, SessionCommand, SlashCommand,
};
pub use theme::{BackgroundMode, Theme, ThemeKind};
pub use keys::{KeyHandler, KeyResult, KeybindingProfile, PromptState, ViMode};
pub use keymap_config::{
    KeyAction, KeyChord, KeymapConfig, KeymapError, KeymapManager, KeymapValidation,
    DEFAULT_LEADER_TIMEOUT_MS, KEYMAP_FILE_NAME,
};
pub use sound::{
    play_cue, play_error, play_turn_complete, ring_bell, ring_bell_to, SoundConfig, SoundCue,
    SoundPlayer, TERMINAL_BELL, TERMINAL_BELL_BYTE,
};
pub use model_picker::{
    default_models, format_context, format_output, format_tokens, pick_model, ModelEntry,
    ModelPicker, ModelPickerResult, ProviderTab,
};
pub use table::{
    get_terminal_width, render_markdown_table, render_markdown_table_with_width, strip_ansi,
    truncate_ansi, visible_width, wrap_ansi, ColumnAlign, ColumnAutoSizer,
    MarkdownTableStreamer, Table, TableBorderStyle, TableTheme,
};
pub use termux::{
    find_termux_tool, get_clipboard, get_clipboard_async, has_termux_api, is_termux,
    set_clipboard, set_clipboard_async, termux_battery_status, termux_config_dir,
    termux_home, termux_notification, termux_prefix, termux_properties_path,
    termux_reload_settings, termux_toast, vibrate, vibrate_async, ExtraKey, ExtraKeysLayout,
    HapticConfig, HapticIntensity, TermuxBatteryInfo, TermuxClipboard, TermuxError,
    TermuxHaptics, TermuxProperties,
};
pub use status::{
    detect_git_branch, detect_git_branch_in, format_branch_badge, format_cost_compact,
    format_duration_compact, format_model_badge, format_token_compact, StatusBar, StatusBarMode,
};
pub use budget::{
    check_critical_95, check_warning_80, evaluate_context_budget, evaluate_session_budget,
    format_progress_bar, format_token_progress_bar, render_budget_banner_ansi,
    render_budget_banner_compact_ansi, render_budget_banner_compact_widget,
    render_budget_banner_widget, render_budget_status_pill_ansi, BannerBoxStyle,
    BudgetAlertTracker, ContextAlert, ContextAlertLevel, ProgressBarConfig, ProgressBarStyle,
    CRITICAL_THRESHOLD, OVERFLOW_THRESHOLD, WARNING_THRESHOLD,
};
pub use diff_view::{
    highlight_tokens_to_spans, run_interactive_diff_viewer, tokenize_line, DiffFile,
    DiffHunk, DiffLine, DiffLineType, DiffViewMode, DiffViewResult, DiffViewState,
    DiffViewerWidget, HunkStatus, SyntaxLanguage, SyntaxToken, TokenKind,
};
pub use voice::{
    create_stt_adapter, render_recording_banner, render_voice_badge, AudioBuffer,
    AudioFormat, AudioLevelMeter, CustomHttpSttAdapter, GroqWhisperAdapter,
    LocalWhisperAdapter, MockSttAdapter, OpenAiWhisperAdapter, SpeechToTextAdapter,
    SttProvider, TranscriptionRequest, TranscriptionResult, TranscriptionSegment,
    VadConfig, VadDetector, VadState, VoiceConfig, VoiceError, VoiceInputState,
    VoiceSession,
};
pub use notify::{
    emit_terminal_notification, emit_terminal_osc_to, format_duration_secs, notify,
    notify_err, notify_error, notify_task, notify_task_complete, notify_turn_complete,
    Notification, NotificationBackend, NotificationConfig, NotificationError,
    NotificationOutcome, NotificationUrgency, TerminalOscProtocol,
};
pub use file_picker::{
    format_file_row, fuzzy_match, fuzzy_search, pick_file, FileEntry, FilePicker,
    FilePickerResult, FileScanner, FileType, FuzzyMatchResult,
};
pub use multicursor::{
    BlockOperations, BlockRange, BufferHistory, BufferSnapshot, CaseTransform, Cursor,
    EditorBuffer, LineJumpHelper, LineJumpTarget, MultiCursorState, MultilineBuffer,
    Position, Selection, SortOptions, TextRange, WordWrapEngine, WrapOptions, WrappedLine,
};
pub use quick::{
    default_quick_actions, format_action_row, pick_quick_action, pick_slash_command,
    QuickAction, QuickActionCategory, QuickActionsMenu, QuickActionResult,
};
pub use side_by_side::{
    compute_intra_line_highlights, is_side_by_side_supported, print_diff, render_adaptive,
    render_diff_with_width, render_side_by_side_ansi, render_unified_ansi, resolve_display_mode,
    AdaptiveDiffConfig, DiffBorderStyle, DiffChangeKind, DiffDisplayMode, DiffStats,
    HighlightRange, SideBySideCell, SideBySideDocument, SideBySideHunk, SideBySideRow,
    SideBySideWidget, DEFAULT_SIDE_BY_SIDE_MIN_WIDTH,
};
pub use colors::{
    ansi16_to_ratatui, ansi256_to_rgb, downsample_color, format_bg_escape, format_fg_escape,
    rgb_to_ansi16, rgb_to_ansi256, ColorCapability, ColorLevel, ColorSupport,
    ANSI_16_RGB, ANSI_RESET,
};
pub use title::{
    clear_terminal_title, format_terminal_title, is_terminal_title_supported, render_osc,
    render_osc0, render_osc2, reset_terminal_title, sanitize_title, set_session_model_status_title,
    set_session_model_title, set_terminal_title, set_terminal_title_to, shorten_model_name,
    OscTerminator, OscType, TerminalTitle, TitleConfig, TitleFormatStyle, TitleGuard, TitleUpdater,
    DEFAULT_APP_NAME, DEFAULT_FALLBACK_TITLE, DEFAULT_MAX_TITLE_LENGTH,
};
pub use doc_render::{
    extract_headings, generate_doc_page, generate_doc_site, generate_embedded_css,
    generate_embedded_js, generate_toc_html, highlight_code_html, markdown_to_html,
    render_doc_page, render_inline_html, slugify, strip_inline_markdown, DocConfig,
    DocConfigBuilder, DocPage, DocTheme, HeadingItem, NavItem, NavSection, ThemeColors,
};
pub use agent_tree::{
    render_tree_ansi, render_tree_diagram, render_tree_plain, AgentTree, AgentTreeAction,
    AgentTreeNode, AgentTreeState, AgentTreeWidget, FlattenedTreeRow, TreeGlyphSet,
    TreeRenderOptions, TreeViewMode,
};
pub use bench_cmd::{
    benchmark_single_provider, discover_benchmark_targets, format_benchmark_json,
    format_rankings_and_recommendation, format_tps_colored, format_troubleshooting_and_unconfigured,
    format_ttft_colored, handle_benchmark_command, parse_benchmark_args, run_async_future,
    run_benchmark_suite, BenchmarkOptions, BenchmarkOutputFormat, BenchmarkRunResult,
    BenchmarkTarget, PerformanceRating, ProviderBenchmarkSummary, DEFAULT_BENCHMARK_MAX_TOKENS,
    DEFAULT_BENCHMARK_PROMPT, DEFAULT_BENCHMARK_ROUNDS, DEFAULT_BENCHMARK_TIMEOUT_SECS,
    DEFAULT_PING_PROMPT,
};
pub use context_view::{
    render_context_bar_ansi, render_context_inspector_ansi, render_context_summary_ansi,
    run_context_inspector, CompactionPreview, ContextBarOptions, ContextBarWidget,
    ContextCategory, ContextCategoryStats, ContextDistribution, ContextInspectorResult,
    ContextInspectorState, ContextInspectorTab, ContextInspectorWidget, MessageContextItem,
    SystemSectionItem, ToolContextItem, DEFAULT_CONTEXT_BAR_WIDTH,
};
pub use prompt_match::{
    current_epoch_secs, default_prompt_templates, extract_next_word_token, frecency_score,
    fuzzy_match_with_config, fuzzy_search_history, highlight_matched_chars,
    is_word_boundary_char, match_token_overlap, render_completion_popup,
    render_reverse_history_search, suggest_completion, suggest_ghost_completion,
    CompletionUIMode, FuzzyMatchResult as PromptFuzzyMatchResult, GhostCompletion,
    MatchKind as PromptMatchKind, PromptAutocompleter, PromptCategory, PromptCompletionState,
    PromptHistoryItem, PromptMatch, PromptMatchConfig,
};
pub use context_warning::{
    render_compact_warning_widget, render_progress_bar_ansi, render_progress_bar_text,
    render_warning_banner_ansi, render_warning_card_ansi, render_warning_compact_ansi,
    render_warning_pill_ansi, render_warning_widget, ContextAlertEvent, ContextAlertSeverity,
    ContextLimitAlert, ContextProgressBarWidget, ContextWarningStyle, ContextWarningTracker,
    ContextWarningWidget, WarningBorderStyle,
    DEFAULT_CRITICAL_THRESHOLD, DEFAULT_NOTICE_THRESHOLD, DEFAULT_OVERFLOW_THRESHOLD,
    DEFAULT_PROGRESS_BAR_WIDTH, DEFAULT_WARNING_THRESHOLD,
};
pub use mouse::{
    disable_mouse_capture, enable_mouse_capture, ClickKind, ListMouseAction, ListMouseHandler,
    MouseButtonKind, MouseCaptureGuard, MouseConfig, MouseTracker, PromptRegion, RegionClassifier,
    ScrollController, TabBarMouseHandler,
};
pub use bench_runner::{
    measure_streaming_provider, render_charts_ansi, render_inspector_ansi,
    render_live_status_ansi, run_interactive_benchmark, BenchPromptPreset, BenchRunnerState,
    BenchRunnerWidget, BenchSortColumn, BenchTab, BenchTargetState, LiveStreamChunk,
    LiveStreamEvent, LiveStreamMetrics, StreamPhase,
};
pub use print_css::{
    generate_page_css, generate_pdf_css, generate_print_css, generate_print_css_rules,
    generate_standalone_print_stylesheet, inject_print_css, optimize_for_pdf, FontSizeScale,
    PageBreakSettings, PageMargin, PageOrientation, PageSize, PdfOptions, PrintFontFamily,
    PrintLineHeight, PrintOptions, PrintTheme, RunningFooterConfig, RunningHeaderConfig,
    DEFAULT_PAGE_CSS, DEFAULT_PRINT_CSS,
};
pub use progress_tree::{
    render_mini_bar, render_progress_summary_line, render_progress_tree_ansi,
    render_progress_tree_plain, FlattenedProgressRow, ProgressGlyphSet, ProgressLogEntry,
    ProgressLogKind, ProgressTokenMetrics, ProgressToolCall, ProgressTreeNode, ProgressTreeOptions,
    ProgressTreeState, ProgressTreeWidget, ProgressViewMode, SubagentFilter,
};
pub use stats_card::{
    format_duration_pretty, render_meter_bar, render_stats_card_ansi, render_stats_card_markdown,
    render_stats_card_plain, render_stats_compact_ansi, SessionStats, SessionStatsBuilder,
    SessionStatsCardState, SessionStatsCardWidget, StatsCardBorderStyle, StatsCardConfig,
    StatsCardLayout, ToolExecutionStat, DEFAULT_CARD_WIDTH, DEFAULT_MAX_TOOLS_DISPLAYED,
    MIN_CARD_WIDTH,
};
pub use banner::{
    apply_diagonal_gradient, apply_horizontal_gradient,
    interpolate_rgb, multi_stop_gradient, print_startup_banner, render_banner,
    render_banner_ansi, render_banner_to, render_compact_banner_ansi,
    render_minimal_banner_ansi, render_oneline_banner_ansi, BannerBoxBorder,
    BannerColorMode, BannerConfig, BannerConfigBuilder, BannerInfo,
    BannerInfoBuilder, BannerStyle, BannerWidget, GradientPreset,
    BANNER_ART_COMPACT, BANNER_ART_CYBER, BANNER_ART_SLEEK, BANNER_ART_SLANT,
    BANNER_ART_STANDARD, DEFAULT_SUBTITLE, QUICK_TIPS,
};
pub use rate_limit_banner::{
    format_time_compact, format_tokens_compact, render_rate_limit_banner_ansi,
    render_rate_limit_banner_widget, render_rate_limit_compact_ansi, render_rate_limit_pill_ansi,
    BackoffStrategy, RateLimitBannerBoxStyle, RateLimitBannerWidget, RateLimitCountdownBarWidget,
    RateLimitInfo, RateLimitKind, RateLimitMiniBannerWidget, RateLimitProgressBarStyle,
    RateLimitStatus, RateLimitTickOutcome, RateLimitTracker, DEFAULT_BANNER_WIDTH as DEFAULT_RATE_LIMIT_BANNER_WIDTH,
    DEFAULT_METER_WIDTH as DEFAULT_RATE_LIMIT_METER_WIDTH, MIN_BANNER_WIDTH as MIN_RATE_LIMIT_BANNER_WIDTH,
};
