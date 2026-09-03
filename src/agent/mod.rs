pub mod advisor;
pub mod bookmark;
pub mod commit_gen;
pub mod compaction;
pub mod consensus;
pub mod correction;
pub mod cost;
pub mod curl;
pub mod export;
pub mod export_jsonl;
pub mod fork;
pub mod heartbeat;
pub mod loop_runner;
pub mod memory;
pub mod mesh;
pub mod metrics;
pub mod plan;
pub mod planner_dag;
pub mod pricing_sync;
pub mod prompt_lib;
pub mod prompts;
pub mod pruner;
pub mod recovery;
pub mod search;
pub mod session;
pub mod session_patch;
pub mod skills;
pub mod snippets;
pub mod subagent;
pub mod tagging;
pub mod throttle;
pub mod tokens;
pub mod trace;
pub mod undo;

pub use advisor::{
    consult_advisors, format_critiques_for_system_prompt, format_critiques_summary, Advisor,
    AdvisorCritique, AdvisorEngine, AdvisorRegistry, RiskLevel,
};
pub use bookmark::{
    bookmark_checkpoint, bookmark_specific_turn, bookmark_turn, bookmarks_dir, clear_bookmarks,
    delete_bookmark, export_bookmarks_json, export_bookmarks_markdown, filter_bookmarks,
    filter_bookmarks_by_tag, fork_from_bookmark, format_bookmark_detail, format_bookmarks_table,
    get_bookmark, get_bookmark_by_turn, get_pinned_turns, handle_bookmark_command,
    import_bookmarks_json, is_turn_pinned, list_bookmarks, load_bookmarks_from_disk, pin_turn,
    recall_bookmark, rename_bookmark, restore_to_bookmark, save_bookmarks_to_disk,
    save_bookmarks_to_session, search_bookmarks, tag_bookmark, unpin_turn, untag_bookmark,
    update_bookmark_note, Bookmark, BookmarkFilter, BookmarkKind, BookmarkRecall, BookmarkSnapshot,
    BOOKMARKS_METADATA_KEY,
};
pub use commit_gen::{
    get_repo_git_diff, infer_scope_from_path, parse_git_diff, CommitFooter, CommitGenerator,
    CommitGeneratorConfig, CommitParseError, CommitType, ConventionalCommit,
    ConventionalCommitBuilder, DiffAnalysis, FileChangeKind, FileDiffSummary,
};
pub use compaction::{
    compact_session, compact_session_with_llm, generate_heuristic_summary, group_into_turns,
    prune_older_tool_outputs, truncate_tool_output, CompactionConfig, CompactionResult,
    CompactionStrategy, Compactor, TurnGroup,
};
pub use consensus::{
    resolve_consensus, resolve_consensus_with_policy, resolve_majority, resolve_risk_weighted,
    resolve_security_veto, resolve_unanimous, AdvisorVote, ConsensusEngine, ConsensusPolicy,
    ConsensusResolution, ConsensusStrategy,
};
pub use correction::{
    clean_path_string, find_fuzzy_file_matches, parse_python_traceback, parse_rust_compiler_errors,
    parse_ts_compiler_errors, CorrectionAttempt, CorrectionConfig, CorrectionEngine,
    CorrectionHistory, CorrectionOutcome, CorrectiveAction, ErrorAnalyzer, ErrorCategory,
    ErrorDiagnosis, PythonTracebackDiagnostic, RustCompilerDiagnostic, TsCompilerDiagnostic,
};
pub use cost::{
    estimate_cost, estimate_session_cost, format_cost_summary, format_usd, format_usd_precise,
    get_model_pricing, CostBreakdown, CostTracker, CostTurnRecord, ModelPricing,
    ModelPricingRegistry,
};
pub use curl::{
    export_all_turns_curl, generate_curl_from_messages, generate_curl_from_session,
    generate_curl_script, generate_latest_turn_curl, generate_reproduction_bundle,
    generate_turn_curl, mask_api_key, provider_env_var_name, ApiKeyVisibility, CurlCommand,
    CurlExportError, CurlExportOptions, CurlFormatting, CurlReproductionBundle, CurlShell,
    TurnRequestScope,
};
pub use export::{
    export_session_html, export_session_html_file, export_session_html_file_with_options,
    export_session_html_with_options, ExportOptions, ExportTheme,
};
pub use export_jsonl::{
    count_tool_calls, estimate_token_count, export_dataset_split, export_session_to_jsonl,
    export_session_to_jsonl_file, export_sessions_to_jsonl, export_sessions_to_jsonl_file,
    extract_thought_blocks, mask_sensitive_credentials, role_to_str, strip_thought_tags,
    validate_jsonl_file, validate_jsonl_string, AlpacaSample, AnthropicMessage, AnthropicSample,
    DatasetSplitResult, DatasetSplitter, DpoPreferenceSample, ExportStats, JsonlExportError,
    JsonlExportOptions, JsonlExporter, JsonlFormat, LlmEvaluationSample, OpenAiChatSample,
    OpenAiExportFunctionCall, OpenAiExportMessage, OpenAiExportToolCall, PromptCompletionSample,
    RawTurnSample, ShareGptMessage, ShareGptSample, ThoughtHandling, TurnSplitStrategy,
    ValidationReport,
};
pub use fork::{
    count_turns, diff_session_branches, extract_turns, fork_session, fork_session_from_str,
    fork_session_in_memory, get_fork_lineage, get_turn, list_branches, preview_branch_tree,
    rewind_session, rewind_session_from_str, rewind_session_in_place, SessionBranchDiff,
    SessionTurn,
};
pub use heartbeat::{
    AutoTickerHandle, BackgroundScannerHandle, HangDiagnosis, HangReason, HealthStatus,
    HeartbeatEvent, HeartbeatMetrics, HeartbeatMonitor, HeartbeatRecord, HeartbeatThresholds,
    MonitoredSubagent, PhaseTransition, RecoveryAction, RecoveryResult, SubagentHeartbeatHandle,
    SubagentHeartbeatTool, SubagentPhase, DEFAULT_DEAD_THRESHOLD, DEFAULT_EVENT_CHANNEL_CAPACITY,
    DEFAULT_HANG_THRESHOLD, DEFAULT_HEARTBEAT_INTERVAL, DEFAULT_MAX_PHASE_DURATION,
    DEFAULT_MAX_STUCK_PROGRESS_DURATION, DEFAULT_STALL_THRESHOLD, DEFAULT_WARNING_THRESHOLD,
    MAX_HEARTBEAT_HISTORY, MAX_PHASE_HISTORY,
};
pub use loop_runner::{AgentEvent, AgentRunner};
pub use memory::{MemoryCategory, MemoryEntry, MemoryStore, MemoryTool};
pub use mesh::{
    topics as mesh_topics, AdvisorReviewHandler, AdvisorReviewRequest, AdvisorReviewResponse,
    AgentInfo as MeshAgentInfo, AgentMesh, AgentRole as MeshAgentRole,
    AgentStatus as MeshAgentStatus, BroadcastMessage as MeshBroadcastMessage,
    BroadcastPayload as MeshBroadcastPayload, DirectMessage as MeshDirectMessage,
    MeshBroadcastTool, MeshClaimResourceTool, MeshError, MeshListPeersTool, MeshPeerChannel,
    MeshQueryPeerTool, MeshRequestReviewTool, PeerQuery, PeerQueryEnvelope, PeerResponse,
    ResourceClaim, SharedFact,
};
pub use metrics::{
    calculate_percentile, RoleAggregateMetrics, SubagentFleetMetrics, SubagentMetricStatus,
    SubagentMetrics, SubagentMetricsCollector, ToolCallSample, ToolUsageMetrics, TurnMetric,
};
pub use plan::{
    AgentStepExecutor, AutoApproveHandler, ChannelConfirmationHandler, CheckpointDecision,
    CheckpointPolicy, CheckpointType, CliPromptHandler, ConfirmationCheckpoint,
    ConfirmationHandler, MockStepExecutor, Phase, PhaseBuilder, PhaseStatus, Plan, PlanBuilder,
    PlanEngine, PlanEvent, PlanGenerator, PlanState, PlanStep, PlanTool, StepBuilder, StepExecutor,
    StepStatus,
};
pub use planner_dag::{
    ContextResolver, DagExecutionConfig, DagExecutionEvent, DagExecutionSummary, DagExecutor,
    DagOverallStatus, DagPlannerError, DagStage, DagTask, DagTaskPriority, DagTaskStatus,
    DagTaskSummary, DecompositionStrategy, MockTaskExecutor, PlanSubagentDagTool, StageStatus,
    SubagentDag, SubagentManagerExecutor, TaskDecomposer, TaskExecutor,
};
pub use pricing_sync::{
    extract_provider_and_model_name, fetch_live_openrouter_pricing, fetch_openrouter_raw,
    format_model_cost_comparison, format_pricing_diff_report, format_pricing_table,
    format_sync_summary, infer_cache_rates, is_pricing_cache_fresh, load_pricing_cache,
    load_stale_pricing_cache, parse_openrouter_model_pricing, parse_openrouter_price_per_million,
    save_pricing_cache, sync_openrouter_pricing, BackgroundSyncHandle, ModelPricingRecord,
    OpenRouterModelPricingRaw, OpenRouterPricingRaw, OpenRouterResponseRaw, PricingCacheEnvelope,
    PricingDiff, PricingSource, PricingSyncConfig, PricingSyncError, PricingSyncStats,
    PricingSynchronizer, DEFAULT_PRICING_CACHE_FILENAME, DEFAULT_PRICING_CACHE_TTL_SECS,
    DEFAULT_PRICING_SYNC_TIMEOUT_SECS, OPENROUTER_MODELS_ENDPOINT, PRICING_CACHE_VERSION,
};
pub use prompt_lib::{
    extract_placeholders, get_curated_builtin_templates, parse_cli_tokens, parse_positional_args,
    project_prompts_dir, prompts_dir, prompts_file, substitute_variables, PromptLibError,
    PromptLibrary, PromptTemplate, PromptTemplateBuilder, PromptVariable,
};
pub use prompts::{
    compose_system_prompt, detect_file_language, detect_project_language, domain_task_prompt,
    domain_task_prompt_with_tokens, general_system_prompt, get_preset_prompt, go_system_prompt,
    is_termux_environment, python_system_prompt, rust_system_prompt, termux_system_prompt,
    typescript_system_prompt, DomainTask, PromptPreset, SystemPromptBuilder,
};
pub use pruner::{
    deduplicate_tool_results_in_place, extract_all_thinking_blocks,
    extract_thinking_blocks_with_custom_tags, group_messages_into_turns, has_thinking_blocks,
    is_error_tool_output, minify_json_tool_output, prune_conversation,
    prune_conversation_with_config, prune_tool_outputs_older_than, strip_all_thinking_blocks,
    strip_thinking_tags, strip_thinking_with_stats, truncate_git_diff_output,
    truncate_tool_output_smart, ConversationPruner, PruneAction, PruneActionType, PruneResult,
    PruneTurn, PrunerConfig, ThinkingPrunePolicy, ToolPrunePolicy, STANDARD_THINKING_TAGS,
};
pub use recovery::{
    check_for_crash, check_for_crash_at_path, clear_recovery_file, format_crash_banner,
    global_recovery_path, handle_recovery_command, load_recovery_state, recovery_file_path,
    resume_session_from_recovery, save_recovery_state_atomic, CompletedToolResult, CrashReport,
    RecoveryError, RecoveryManager, RecoveryState, ResumeResult, ResumeStrategy, TurnPhase,
    TurnRecoveryGuard, FUSION_DIR_NAME, RECOVERY_FILE_NAME, RECOVERY_SCHEMA_VERSION,
};
pub use search::{
    search_in_sessions, search_sessions, search_sessions_dir, IndexedSessionMeta, MatchedField,
    MessageMatch, SearchMode, SearchPosting, SearchQuery, SearchReport, SessionSearchIndex,
    SessionSearchResult,
};
pub use session::{Session, SessionSummary, TokenStats, TokenUsage};
pub use session_patch::{
    export_session_patch, export_session_patch_default, export_session_patch_file,
    export_session_patch_string, format_session_patch_summary, FileEditKind, PatchFileStats,
    SessionFilePatch, SessionPatch, SessionPatchAggregator, SessionPatchBuilder,
    SessionPatchOptions, SessionPatchSummary, DEFAULT_CONTEXT_RADIUS,
    DEFAULT_SESSION_PATCH_FILENAME,
};
pub use skills::{Skill, SkillLoader, SkillMatch, SkillMetadata, SkillRegistry, SkillSource};
pub use snippets::{
    clear_snippets, delete_snippet, delete_snippet_from, detect_code_language,
    export_snippets_json, extract_code_blocks, extract_last_code_snippet, format_snippet_detail,
    format_snippet_help, format_snippet_table, get_snippet, handle_snippet_command,
    import_snippets_json, insert_snippet, is_valid_snippet_name, list_snippets, list_snippets_from,
    load_snippet, load_snippet_from, project_snippets_dir, recall_snippet, sanitize_snippet_name,
    save_snippet, save_snippet_to, snippet_file_path, snippets_dir, Snippet, SnippetError,
    SnippetManager,
};
pub use subagent::{
    run_subagent, SpawnBatchSubagentsTool, SpawnSubagentTool, Subagent, SubagentHandle,
    SubagentInfo, SubagentManager, SubagentProgress, SubagentResult, SubagentRole, SubagentStatus,
    SubagentTask,
};
pub use tagging::{
    add_tag, add_tag_with_details, add_tags, clear_tags, deterministic_tag_color,
    extract_tags_from_session, filter_sessions_by_tag, filter_sessions_in_dir,
    filter_sessions_multi, format_active_session_tags, format_global_tags_table, format_tag_badge,
    format_tag_badge_str, format_tag_help, format_tag_stats_report, format_tagged_sessions_table,
    get_session_tag_names, get_session_tags, handle_tag_command, has_tag, list_all_tags,
    list_all_tags_in_dir, normalize_tag_name, remove_tag, remove_tags, rename_tag,
    set_session_tags, tag_ansi_bg, tag_ansi_fg, tag_saved_session, untag_saved_session,
    validate_tag_name, SessionTag, SessionTagCollection, SessionTagManager, TagFilterMode,
    TagFilterQuery, TagFrequency, TagStatsReport, TaggedSessionSummary, TaggingError,
    DEFAULT_TAG_COLOR, LEGACY_TAGS_KEY, MAX_TAG_NAME_LENGTH, MIN_TAG_NAME_LENGTH,
    TAGS_METADATA_KEY, TAG_COLOR_PALETTE,
};
pub use throttle::{
    enforce_throttle_async, new_shared_throttle_engine, QuotaLevel, QuotaType, ReservationTicket,
    SharedThrottleEngine, ThrottleConfig, ThrottleDecision, ThrottleEngine, ThrottleError,
    ThrottleMetrics, ThrottlePolicy, ThrottleStatusReport, TokenBucket, TokenQuotaConfig,
    TokenQuotaManager, TurnRateLimitConfig, TurnRateLimiter,
};
pub use tokens::{
    estimate_message_tokens, estimate_messages_tokens, estimate_messages_tokens_with_system,
    estimate_text_tokens, estimate_tokens, estimate_tokens_simple, estimate_tool_call_tokens,
    estimate_tool_definition_tokens, estimate_tools_tokens, format_token_count,
    is_context_overflow, model_context_limit, suggest_truncation_window, BudgetStatus,
    ContextBreakdown, ContextBudget, TokenCount, TokenTracker, APPROX_CHARS_PER_TOKEN,
    DEFAULT_CONTEXT_WINDOW,
};
pub use trace::{
    export_trace_markdown, generate_trace, handle_trace_command, save_trace_file, traces_dir,
    DiagnosticTrace, GitInfo, RedactionAudit, RedactionCategory, SanitizedTranscriptMessage,
    SessionMetadata, SystemInfo, ToolExecutionRecord, TraceExportOptions, TraceRedactor,
};
pub use undo::{
    extract_target_paths, format_checkpoints_table, format_redo_report, format_undo_report,
    new_shared_checkpoint_manager, resolve_target_path, Checkpoint, CheckpointManager,
    CheckpointStatus, CheckpointSummary, FileActionTaken, FileChangeType, FileDiff, FileSnapshot,
    FileState, RedoResult, SharedCheckpointManager, UndoResult, DEFAULT_MAX_CHECKPOINTS,
    MAX_SNAPSHOT_FILE_SIZE,
};
