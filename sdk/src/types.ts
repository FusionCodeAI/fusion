/**
 * @fusion/sdk — TypeScript Definitions
 * Comprehensive type definitions for:
 * 1. Agent Client Protocol (ACP) JSON-RPC 2.0 messages & lifecycle
 * 2. Agent Mesh, Peer-to-Peer RPC, Subagents & Multi-Agent Advisors
 * 3. Session State, Message History, Tool Definitions, VFS, and Configuration
 * 4. WebAssembly bindings and event streaming callbacks
 *
 * @packageDocumentation
 */

// ============================================================================
// 1. JSON-RPC 2.0 Base Protocol Types & Error Codes
// ============================================================================

/**
 * Protocol version string for JSON-RPC 2.0 messages.
 */
export type JsonRpcVersion = '2.0';

/**
 * Request identifier for JSON-RPC 2.0 calls.
 * Per specification, request IDs may be strings, numbers, or null.
 */
export type RequestId = string | number | null;

/**
 * Standard Agent Client Protocol (ACP) integer version number.
 */
export const ACP_PROTOCOL_VERSION = 1;

/**
 * Standard JSON-RPC 2.0 and ACP Error Codes.
 */
export const JSON_RPC_ERROR_CODES = {
  /** Invalid JSON was received by the server. */
  PARSE_ERROR: -32700,
  /** The JSON sent is not a valid Request object. */
  INVALID_REQUEST: -32600,
  /** The method does not exist / is not available. */
  METHOD_NOT_FOUND: -32601,
  /** Invalid method parameter(s). */
  INVALID_PARAMS: -32602,
  /** Internal JSON-RPC error. */
  INTERNAL_ERROR: -32603,
  /** Server has not been initialized with `initialize` handshake. */
  SERVER_NOT_INITIALIZED: -32002,
  /** The requested session identifier was not found. */
  SESSION_NOT_FOUND: -32001,
  /** The prompt turn or request was cancelled by the client. */
  REQUEST_CANCELLED: -32000,
  /** Tool execution failure. */
  TOOL_EXECUTION_ERROR: -32010,
  /** MCP server communication error. */
  MCP_ERROR: -32020,
  /** Rate limit exceeded. */
  RATE_LIMIT_EXCEEDED: -32030,
  /** Authentication or permission error. */
  AUTH_ERROR: -32040,
} as const;

/**
 * Union of standard error code numeric values.
 */
export type JsonRpcErrorCode =
  (typeof JSON_RPC_ERROR_CODES)[keyof typeof JSON_RPC_ERROR_CODES] | number;

/**
 * JSON-RPC 2.0 Error Object.
 */
export interface JsonRpcError<TData = unknown> {
  /** Numeric error code indicating the error category. */
  code: JsonRpcErrorCode;
  /** Short, human-readable summary of the error. */
  message: string;
  /** Optional structured debug data or stack trace details. */
  data?: TData;
}

/**
 * Incoming JSON-RPC 2.0 Request Object.
 */
export interface JsonRpcRequest<TParams = unknown> {
  /** Must be exactly "2.0". */
  jsonrpc: JsonRpcVersion;
  /** Unique request ID. Optional for notifications. */
  id?: RequestId;
  /** Name of the RPC method to invoke. */
  method: string;
  /** Method-specific parameter payload. */
  params?: TParams;
}

/**
 * Outgoing JSON-RPC 2.0 Response Object (Success or Error).
 */
export interface JsonRpcResponse<TResult = unknown, TErrorData = unknown> {
  /** Must be exactly "2.0". */
  jsonrpc: JsonRpcVersion;
  /** Matching request ID from the initiating request. */
  id: RequestId;
  /** Result payload on successful execution. */
  result?: TResult;
  /** Error object if execution failed. */
  error?: JsonRpcError<TErrorData>;
}

/**
 * Outgoing JSON-RPC 2.0 Notification Object (e.g. streaming updates).
 */
export interface JsonRpcNotification<TParams = unknown> {
  /** Must be exactly "2.0". */
  jsonrpc: JsonRpcVersion;
  /** Notification method name (e.g. "session/update"). */
  method: string;
  /** Notification payload. */
  params: TParams;
}

// ============================================================================
// 2. Agent Client Protocol (ACP) Handshake & Capabilities
// ============================================================================

/**
 * Client filesystem capabilities advertised during initialization.
 */
export interface FsCapabilities {
  /** Whether the client supports reading text files directly. */
  readTextFile?: boolean;
  /** Whether the client supports writing text files directly. */
  writeTextFile?: boolean;
}

/**
 * Client session capabilities advertised during initialization.
 */
export interface ClientSessionCapabilities {
  /** Whether the client supports streaming chunk notifications. */
  streaming?: boolean;
}

/**
 * Capabilities advertised by the connecting client editor (e.g. VSCode, Zed, Neovim).
 */
export interface ClientCapabilities {
  /** Filesystem access capabilities. */
  fs?: FsCapabilities;
  /** Whether the client provides interactive terminal integration. */
  terminal?: boolean;
  /** Session management capabilities. */
  session?: ClientSessionCapabilities;
}

/**
 * Information describing the connecting client.
 */
export interface ClientInfo {
  /** Client application name (e.g. "zed", "vscode", "fusion-cli"). */
  name: string;
  /** Client version string (e.g. "1.0.0"). */
  version?: string;
}

/**
 * Parameters for the ACP `initialize` method.
 */
export interface InitializeRequest {
  /** Protocol version number requested by client. */
  protocolVersion: number;
  /** Client capabilities. */
  clientCapabilities: ClientCapabilities;
  /** Client application metadata. */
  clientInfo?: ClientInfo;
}

/**
 * Modality and prompt capabilities supported by the Fusion agent.
 */
export interface PromptCapabilities {
  /** Image input support. */
  image: boolean;
  /** Audio input support. */
  audio: boolean;
  /** Embedded workspace resources support. */
  embeddedResources: boolean;
}

/**
 * MCP (Model Context Protocol) capabilities supported by the agent.
 */
export interface McpCapabilities {
  /** Whether the agent supports dynamically connecting to MCP servers. */
  servers: boolean;
}

/**
 * Capabilities supported by the Fusion agent backend.
 */
export interface AgentCapabilities {
  /** Whether the agent supports resuming existing sessions. */
  loadSession: boolean;
  /** Prompt modality capabilities. */
  promptCapabilities: PromptCapabilities;
  /** MCP integration capabilities. */
  mcpCapabilities?: McpCapabilities;
  /** Terminal execution capabilities. */
  terminal?: boolean;
}

/**
 * Metadata identifying the Fusion agent.
 */
export interface AgentInfo {
  /** Agent engine name (default: "fusion"). */
  name: string;
  /** Agent engine version (e.g. "0.3.0"). */
  version: string;
  /** Description of the agent's features and architecture. */
  description?: string;
}

/**
 * Authentication method supported by the agent server.
 */
export interface AuthMethod {
  /** Unique auth method identifier. */
  id: string;
  /** Display name. */
  name: string;
  /** Detailed description. */
  description?: string;
}

/**
 * Result returned by the agent for the `initialize` method.
 */
export interface InitializeResult {
  /** Negotiated protocol version. */
  protocolVersion: number;
  /** Capabilities advertised by the agent. */
  agentCapabilities: AgentCapabilities;
  /** Information identifying the agent. */
  agentInfo: AgentInfo;
  /** Supported authentication methods (if any). */
  authMethods?: AuthMethod[];
}

// ============================================================================
// 3. ACP Session Lifecycle & Model Descriptors
// ============================================================================

/**
 * Unique session identifier string (UUID v4 format).
 */
export type SessionId = string;

/**
 * Descriptor for an LLM model advertised to the client.
 */
export interface ModelInfo {
  /** Unique model identifier (e.g. "anthropic/claude-3.5-sonnet", "deepseek/deepseek-chat"). */
  id: string;
  /** Display name (e.g. "Claude 3.5 Sonnet"). */
  name: string;
  /** Provider identifier (e.g. "openrouter", "anthropic", "openai", "ollama"). */
  provider: string;
  /** Whether this is the default active model for the agent. */
  isDefault?: boolean;
  /** Maximum context window size in tokens. */
  contextLength?: number;
  /** Cost and pricing tier description. */
  pricing?: string;
  /** Brief description of model strengths and specializations. */
  description?: string;
}

/**
 * Parameters for creating a new session via `session/new`.
 */
export interface NewSessionRequest {
  /** Initial working directory path. */
  cwd?: string;
  /** Initial MCP server configurations. */
  mcpServers?: Record<string, unknown>[];
  /** Preferred model identifier. */
  model?: string;
  /** Preferred provider backend. */
  provider?: string;
  /** System prompt override. */
  systemPrompt?: string;
}

/**
 * Result returned when a new session is created.
 */
export interface NewSessionResult {
  /** Newly allocated unique session ID. */
  sessionId: SessionId;
  /** Available models for this session. */
  models?: ModelInfo[];
}

/**
 * Parameters for loading or resuming an existing session via `session/load`.
 */
export interface LoadSessionRequest {
  /** Target session ID to load. */
  sessionId: SessionId;
}

/**
 * Result returned when a session is loaded.
 */
export interface LoadSessionResult {
  /** Loaded session ID. */
  sessionId: SessionId;
  /** Currently active model in the loaded session. */
  activeModel: string;
  /** Total number of messages in the session transcript. */
  messageCount: number;
  /** Optional session title. */
  title?: string;
}

/**
 * Parameters for listing sessions via `session/list`.
 */
export interface ListSessionsRequest {
  /** Maximum number of sessions to return. */
  limit?: number;
  /** Pagination offset. */
  offset?: number;
}

/**
 * Summary descriptor of a single conversation session.
 */
export interface SessionSummaryItem {
  /** Unique session ID. */
  sessionId: SessionId;
  /** ISO-8601 creation timestamp. */
  createdAt: string;
  /** ISO-8601 last update timestamp. */
  updatedAt: string;
  /** Active model identifier. */
  model: string;
  /** Number of messages in transcript. */
  messageCount: number;
  /** Short text preview of recent activity. */
  preview: string;
  /** Optional human-readable session title. */
  title?: string;
}

/**
 * Result returned for `session/list`.
 */
export interface ListSessionsResult {
  /** List of session summaries. */
  sessions: SessionSummaryItem[];
}

/**
 * Parameters for closing a session via `session/close`.
 */
export interface CloseSessionRequest {
  /** Target session ID to close. */
  sessionId: SessionId;
}

/**
 * Parameters for cancelling an ongoing turn via `session/cancel`.
 */
export interface CancelSessionRequest {
  /** Target session ID with an active running turn. */
  sessionId: SessionId;
}

// ============================================================================
// 4. ACP Prompt Dispatching, Content Blocks & Stop Reasons
// ============================================================================

/**
 * Content type descriptor for multimodal content blocks.
 */
export type ContentType =
  | 'text'
  | 'image'
  | 'audio'
  | 'resource'
  | 'diff'
  | 'custom'
  | (string & {});

/**
 * Structured content block within an ACP prompt or response.
 */
export interface ContentBlock {
  /** Type of content (e.g. "text", "image", "resource"). Defaults to "text". */
  type: ContentType;
  /** Text content for text blocks. */
  text?: string;
  /** Base64-encoded data for binary blocks (images/audio). */
  data?: string;
  /** MIME type for binary payloads (e.g. "image/png", "text/plain"). */
  mimeType?: string;
  /** Resource URI for embedded files or external resources. */
  uri?: string;
}

/**
 * Flexible user prompt representation: plain string, single block, or array of blocks.
 */
export type PromptInput = string | ContentBlock | ContentBlock[];

/**
 * Parameters for executing an agent turn via `session/prompt`.
 */
export interface PromptRequest {
  /** Target session ID. */
  sessionId: SessionId;
  /** User prompt text or structured blocks. */
  prompt: PromptInput;
}

/**
 * Reason the agent stopped processing a prompt turn.
 */
export type StopReason =
  | 'end_turn'
  | 'max_tokens'
  | 'max_turn_requests'
  | 'refusal'
  | 'cancelled'
  | 'error';

/**
 * Token usage metadata returned in prompt responses.
 */
export interface TokenStatsInfo {
  /** Number of prompt / input tokens. */
  promptTokens?: number;
  /** Number of completion / output tokens. */
  completionTokens?: number;
  /** Total tokens processed in turn. */
  totalTokens?: number;
  /** Number of cached prompt tokens. */
  cachedTokens?: number;
  /** Estimated cost in USD. */
  costUsd?: number;
}

/**
 * Final response returned after completing a `session/prompt` turn.
 */
export interface PromptResponse {
  /** The reason generation stopped. */
  stopReason: StopReason;
  /** Generated content blocks. */
  content?: ContentBlock[];
  /** Token usage statistics for this turn. */
  stats?: TokenStatsInfo;
}

// ============================================================================
// 5. ACP Streaming Updates (`session/update`)
// ============================================================================

/**
 * Structured message content wrapper for agent chunk updates.
 */
export interface AgentMessageContent {
  /** Message role (usually "assistant"). */
  role: string;
  /** Array of content blocks. */
  content: ContentBlock[];
}

/**
 * Discriminator kind for `SessionUpdate` notifications.
 */
export type SessionUpdateKind =
  | 'agent_message_chunk'
  | 'agent_thought_chunk'
  | 'tool_call'
  | 'tool_status'
  | 'tool_call_result'
  | 'advisor_started'
  | 'advisor_critique'
  | 'token_stats'
  | 'status'
  | 'plan'
  | 'subagent_update';

/**
 * Discriminated union of streaming update events dispatched during turn execution.
 */
export type SessionUpdate =
  | {
      kind: 'agent_message_chunk';
      content: AgentMessageContent;
      index?: number;
      isFirst?: boolean;
      isLast?: boolean;
    }
  | {
      kind: 'agent_thought_chunk';
      thought: string;
      index?: number;
      elapsedMs?: number;
    }
  | {
      kind: 'tool_call';
      callId: string;
      name: string;
      args: Record<string, unknown>;
      status?: string;
    }
  | {
      kind: 'tool_status';
      callId: string;
      name: string;
      status: string;
      progress?: number;
      partialOutput?: string;
    }
  | {
      kind: 'tool_call_result';
      callId: string;
      name: string;
      output: string;
      success: boolean;
      durationMs?: number;
      error?: string;
    }
  | {
      kind: 'advisor_started';
      advisor: string;
      role: string;
    }
  | {
      kind: 'advisor_critique';
      advisor: string;
      approved: boolean;
      critique: string;
      role?: string;
      severity?: string;
      suggestions?: string[];
    }
  | {
      kind: 'token_stats';
      promptTokens: number;
      completionTokens: number;
      totalTokens: number;
      cachedTokens?: number;
      tokensPerSecond?: number;
    }
  | {
      kind: 'status';
      message: string;
      level?: 'info' | 'warn' | 'error' | 'success' | string;
    }
  | {
      kind: 'plan';
      steps: string[];
    }
  | {
      kind: 'subagent_update';
      name: string;
      status: string;
      task?: string;
      output?: string;
    };

/**
 * Parameter payload for `session/update` JSON-RPC notifications.
 */
export interface SessionUpdateParams {
  /** Session ID emitting the update. */
  sessionId: SessionId;
  /** Detailed update payload. */
  update: SessionUpdate;
}

// ============================================================================
// 6. Agent Mesh & Peer-to-Peer RPC Types
// ============================================================================

/**
 * Specialized roles assumed by agents within the mesh coordination fabric.
 */
export type AgentRole =
  | 'scout'
  | 'coder'
  | 'tester'
  | 'reviewer'
  | 'advisor'
  | 'orchestrator'
  | 'general'
  | (string & {});

/**
 * Dynamic execution status of an agent within the mesh.
 */
export type AgentStatus =
  | { state: 'idle' }
  | { state: 'active'; task: string }
  | { state: 'progress'; step: number; total?: number; message: string }
  | { state: 'blocked'; reason: string; waitingFor?: string }
  | { state: 'reviewing'; subject: string }
  | { state: 'completed'; result?: string }
  | { state: 'failed'; error: string }
  | { state: 'terminated' };

/**
 * Metadata and registration info for a peer agent registered in the mesh.
 */
export interface MeshAgentInfo {
  /** Unique agent identifier (e.g. "Scout-1", "Coder-2", "ArchitectureAdvisor"). */
  id: string;
  /** Role assigned to the agent. */
  role: AgentRole;
  /** Brief description of the agent's function. */
  description: string;
  /** Current execution status. */
  status: AgentStatus;
  /** When the agent registered with the mesh (ISO-8601). */
  registeredAt: string;
  /** Last recorded heartbeat or activity timestamp (ISO-8601). */
  lastActive: string;
  /** Optional capability tags (e.g. ["rust", "diff", "security", "filesystem"]). */
  capabilities: string[];
}

/**
 * Standard pub-sub broadcast topics supported by the agent mesh.
 */
export type MeshTopic =
  | 'status'
  | 'discovery'
  | 'alert'
  | 'coordination'
  | '*'
  | (string & {});

/**
 * Payload carried by a pub-sub broadcast message across the mesh.
 */
export type BroadcastPayload =
  | {
      type: 'status';
      status: AgentStatus;
    }
  | {
      type: 'discovery';
      topic: string;
      findings: string;
      fileReferences: string[];
    }
  | {
      type: 'alert';
      severity: 'info' | 'warn' | 'error' | 'critical' | string;
      message: string;
    }
  | {
      type: 'fact_update';
      key: string;
      value: unknown;
    }
  | {
      type: 'custom';
      kind: string;
      data: unknown;
    };

/**
 * Message broadcast to all interested peers across the agent mesh.
 */
export interface BroadcastMessage {
  /** Unique message identifier (UUID v4). */
  id: string;
  /** Sender agent ID. */
  sender: string;
  /** Topic for broadcast routing (e.g. "status", "discovery", "alert"). */
  topic: MeshTopic;
  /** The message payload. */
  payload: BroadcastPayload;
  /** ISO-8601 timestamp string when broadcast was sent. */
  timestamp: string;
}

/**
 * Point-to-point direct message sent from one peer agent to another.
 */
export interface DirectMessage {
  /** Unique message identifier (UUID v4). */
  id: string;
  /** Sender agent ID. */
  from: string;
  /** Recipient agent ID. */
  to: string;
  /** Subject or intent of the message. */
  subject: string;
  /** Textual content or body. */
  content: string;
  /** Optional structured JSON payload. */
  payload?: unknown;
  /** Optional parent message ID for conversation threading. */
  replyTo?: string;
  /** ISO-8601 timestamp string. */
  timestamp: string;
}

/**
 * Direct RPC query request sent from one peer agent to another awaiting an answer.
 */
export interface PeerQuery {
  /** Unique query identifier (UUID v4). */
  queryId: string;
  /** Sender agent ID. */
  from: string;
  /** Recipient agent ID. */
  to: string;
  /** Query question or request text. */
  query: string;
  /** Optional contextual details or file snippets. */
  context?: string;
  /** ISO-8601 timestamp when query was dispatched. */
  timestamp: string;
}

/**
 * An answer returned in response to a `PeerQuery`.
 */
export interface PeerResponse {
  /** Correlated query identifier. */
  queryId: string;
  /** Responder agent ID. */
  from: string;
  /** Original requester agent ID. */
  to: string;
  /** Answer content text. */
  answer: string;
  /** Whether the query was answered successfully. */
  success: boolean;
  /** Optional structured result data. */
  data?: unknown;
  /** ISO-8601 timestamp when response was generated. */
  timestamp: string;
}

/**
 * Query envelope delivering a query to a recipient peer along with response callbacks.
 */
export interface PeerQueryEnvelope {
  /** The incoming query. */
  query: PeerQuery;
  /** Send a successful response back to the requester. */
  respond(answer: string, data?: unknown): void;
  /** Send a failure response back to the requester. */
  fail(error: string): void;
}

// ============================================================================
// 7. Subagent Types & Lifecycle Events
// ============================================================================

/**
 * Specialized roles for worker subagents.
 */
export type SubagentRole =
  | 'scout'
  | 'coder'
  | 'tester'
  | 'reviewer'
  | 'general'
  | { name: string; prompt: string };

/**
 * Execution status of a subagent.
 */
export type SubagentStatus =
  | 'pending'
  | 'running'
  | 'paused'
  | 'completed'
  | 'failed'
  | 'cancelled';

/**
 * Progress and communication events emitted throughout a subagent's lifecycle.
 */
export type SubagentProgress =
  | {
      type: 'started';
      id: string;
      name: string;
      role: SubagentRole;
      task: string;
    }
  | {
      type: 'turn_started';
      id: string;
      turn: number;
      maxTurns: number;
    }
  | {
      type: 'thinking';
      id: string;
      delta: string;
    }
  | {
      type: 'message';
      id: string;
      content: string;
    }
  | {
      type: 'tool_started';
      id: string;
      tool: string;
      args: Record<string, unknown>;
    }
  | {
      type: 'tool_completed';
      id: string;
      tool: string;
      output: string;
      success: boolean;
    }
  | {
      type: 'completed';
      id: string;
      output: string;
      turnsTaken: number;
    }
  | {
      type: 'failed';
      id: string;
      error: string;
    }
  | {
      type: 'cancelled';
      id: string;
    };

// ============================================================================
// 8. Advisor Reviews, Critiques & Risk Assessment
// ============================================================================

/**
 * Assessed risk level from an advisor evaluation.
 */
export type RiskLevel = 'low' | 'medium' | 'high' | 'critical';

/**
 * Domain-specific critique returned by an advisor agent.
 */
export interface AdvisorCritique {
  /** Advisor identifier (e.g. "ArchitectureAdvisor", "SecurityAdvisor", "CodeReviewAdvisor"). */
  advisor: string;
  /** Whether the advisor approved the proposed action. */
  approved: boolean;
  /** Assessed risk level. */
  riskLevel: RiskLevel;
  /** Crisp 1-3 sentence explanation of advice, risks, or validation. */
  critique: string;
  /** Actionable recommendations or alternatives. */
  suggestions?: string[];
  /** Optional severity override string. */
  severity?: string;
  /** Advisor domain role description. */
  role?: string;
}

/**
 * Request submitted by a subagent for architectural, security, or code quality review.
 */
export interface AdvisorReviewRequest {
  /** Unique review ticket ID (UUID v4). */
  requestId: string;
  /** Subagent requesting the review. */
  requester: string;
  /** Target advisor name, or omitted to consult all default advisors. */
  advisor?: string;
  /** Brief subject of what is being reviewed. */
  subject: string;
  /** The planned action, code diff, command, or proposal. */
  diffOrPlan: string;
  /** Optional context or motivation. */
  context?: string;
  /** ISO-8601 timestamp when requested. */
  timestamp: string;
}

/**
 * Consolidated advisor critique and assessment returned to the subagent.
 */
export interface AdvisorReviewResponse {
  /** Correlated review ticket ID. */
  requestId: string;
  /** Whether all consulting advisors approved the proposed action. */
  approved: boolean;
  /** Individual critiques from each consulting advisor. */
  critiques: AdvisorCritique[];
  /** Highest risk level assessed among all critiques. */
  highestRisk: RiskLevel;
  /** Textual summary of critiques and suggestions. */
  summary: string;
  /** ISO-8601 timestamp when review completed. */
  reviewedAt: string;
}

// ============================================================================
// 9. Tool Definitions, Execution & Schemas
// ============================================================================

/**
 * JSON Schema definition for a single tool parameter property.
 */
export interface ToolParameterProperty {
  /** Data type of the property. */
  type: 'string' | 'number' | 'integer' | 'boolean' | 'array' | 'object' | string;
  /** Human-readable description explaining what the argument does. */
  description?: string;
  /** Allowed enumeration values for string properties. */
  enum?: string[];
  /** Schema for array item elements. */
  items?: ToolParameterProperty | Record<string, unknown>;
  /** Nested object properties. */
  properties?: Record<string, ToolParameterProperty | Record<string, unknown>>;
  /** Required child property names. */
  required?: string[];
  /** Default value if omitted. */
  default?: unknown;
  /** Minimum value for numeric properties. */
  minimum?: number;
  /** Maximum value for numeric properties. */
  maximum?: number;
}

/**
 * JSON Schema specification for tool parameters.
 */
export interface ToolParameterSchema {
  /** Top-level schema type (almost always "object"). */
  type: 'object';
  /** Map of parameter names to their schemas. */
  properties: Record<string, ToolParameterProperty | Record<string, unknown>>;
  /** List of required parameter keys. */
  required?: string[];
  /** Whether extra unspecified properties are accepted. */
  additionalProperties?: boolean;
  /** Extra JSON Schema keywords. */
  [key: string]: unknown;
}

/**
 * Complete definition of a tool made available to the LLM.
 */
export interface ToolDefinition {
  /** Tool name (e.g. "read", "write", "edit", "grep", "glob", "bash"). */
  name: string;
  /** Description explaining what the tool does and when to call it. */
  description: string;
  /** JSON Schema parameters specification. */
  parameters: ToolParameterSchema | Record<string, unknown>;
}

/**
 * Extended metadata describing a registered tool.
 */
export interface ToolInfo {
  /** Tool name identifier. */
  name: string;
  /** Short description. */
  description: string;
  /** Tool parameter schema. */
  parameters?: ToolParameterSchema | Record<string, unknown>;
  /** Functional category. */
  category?: 'filesystem' | 'search' | 'execution' | 'analysis' | 'mcp' | 'mesh' | 'custom' | string;
  /** Whether calling this tool requires explicit human/advisor confirmation. */
  requiresApproval?: boolean;
}

/**
 * Single tool call execution request emitted by the LLM.
 */
export interface ToolCall {
  /** Unique ID for the tool invocation (e.g. "call_12345"). */
  id: string;
  /** Name of the tool (e.g. "read", "write", "edit", "grep", "glob", "bash"). */
  name: string;
  /** Serialized JSON argument payload string. */
  arguments: string;
}

/**
 * Tool execution result recorded in conversation history.
 */
export interface ToolResult {
  /** Matching tool call ID. */
  toolCallId: string;
  /** Standard output or error message from execution. */
  output: string;
  /** Whether the tool executed successfully. */
  success?: boolean;
  /** Name of the executed tool. */
  name?: string;
  /** Execution duration in milliseconds. */
  durationMs?: number;
}

// ============================================================================
// 10. Message History, Session State & Token Statistics
// ============================================================================

/**
 * Supported LLM Provider Backends in Fusion.
 */
export type ProviderType =
  | 'openrouter'
  | 'anthropic'
  | 'openai'
  | 'ollama'
  | 'custom'
  | (string & {});

/**
 * Message Role in conversation history.
 */
export type MessageRole = 'system' | 'user' | 'assistant' | 'tool';

/**
 * Chat message stored in session history and sent to LLM providers.
 */
export interface Message {
  /** Role of message sender. */
  role: MessageRole;
  /** Text content of the message. */
  content: string;
  /** Optional reasoning/thinking trace (e.g. for DeepSeek R1 / Claude 3.7 Thinking). */
  reasoning_content?: string;
  /** Optional tool calls emitted by the assistant. */
  tool_calls?: ToolCall[];
  /** Optional tool call ID when role is 'tool'. */
  tool_call_id?: string;
  /** Optional sender name for multi-agent transcripts. */
  name?: string;
  /** ISO-8601 or UNIX timestamp string. */
  timestamp?: string;
}

/**
 * Token usage statistics across the agent session.
 */
export interface TokenStats {
  /** Total prompt / input tokens processed. */
  prompt_tokens: number;
  /** Total completion / output tokens generated. */
  completion_tokens: number;
  /** Total tokens accumulated. */
  total_tokens: number;
  /** Cached input tokens read from KV cache (if supported by provider). */
  cached_tokens?: number;
  /** Estimated cost in USD. */
  estimated_cost_usd?: number;
}

/**
 * High-level session statistics for UI display.
 */
export interface SessionStats {
  /** Cumulative prompt tokens. */
  promptTokens: number;
  /** Cumulative completion tokens. */
  completionTokens: number;
  /** Cumulative total tokens. */
  totalTokens: number;
  /** Number of executed conversation turns. */
  turnCount: number;
  /** Number of tool calls performed. */
  toolCallCount?: number;
  /** Total execution duration in milliseconds. */
  durationMs?: number;
  /** Estimated cost in USD. */
  estimatedCostUsd?: number;
}

/**
 * Represents a single file in the Virtual File System (VFS).
 */
export interface VirtualFile {
  /** Relative file path. */
  path: string;
  /** Text content. */
  content: string;
  /** File size in bytes. */
  size?: number;
  /** ISO-8601 last modified timestamp. */
  modifiedAt?: string;
}

/**
 * Full state of the Virtual File System (VFS).
 */
export interface VirtualFileSystemState {
  /** Map of file paths to their string contents. */
  files: Record<string, string>;
}

/**
 * In-memory representation of an agent session state.
 */
export interface SessionState {
  /** Unique session ID. */
  id: SessionId;
  /** Currently active model identifier. */
  activeModel: string;
  /** System prompt override. */
  systemPrompt?: string;
  /** Message history. */
  messages: Message[];
  /** Token usage statistics. */
  tokenStats: TokenStats;
  /** Turn counter. */
  turnCounter: number;
  /** ISO-8601 creation timestamp. */
  createdAt?: string;
  /** ISO-8601 last modified timestamp. */
  updatedAt?: string;
}

// ============================================================================
// 11. Configuration Options & Turn Execution Options
// ============================================================================

/**
 * Configuration options for initializing a Fusion Agent.
 */
export interface FusionConfig {
  /** Default provider to use for inference. Defaults to "openrouter". */
  default_provider?: ProviderType;
  /** Default model identifier (e.g. "anthropic/claude-3.5-sonnet", "deepseek/deepseek-chat"). */
  default_model?: string;
  /** System prompt override for the session. */
  system_prompt?: string;
  /** Sampling temperature (0.0 to 2.0). Defaults to 0.2. */
  default_temperature?: number;
  /** Maximum generation tokens. Defaults to 4096. */
  max_tokens?: number;
  /** OpenRouter API Key. */
  openrouter_api_key?: string;
  /** Anthropic API Key. */
  anthropic_api_key?: string;
  /** OpenAI API Key. */
  openai_api_key?: string;
  /** Ollama Base URL. Defaults to "http://localhost:11434". */
  ollama_base_url?: string;
  /** Enable multi-agent advisor critiques (e.g. Architect, Security, Performance). Defaults to true. */
  advisors_enabled?: boolean;
  /** Custom model endpoint URL for reverse proxying or self-hosted LLMs. */
  custom_base_url?: string;
  /** Optional custom headers to send with API requests. */
  custom_headers?: Record<string, string>;
  /** Request timeout in milliseconds. */
  timeout_ms?: number;
  /** Extra arbitrary configuration options. */
  [key: string]: unknown;
}

/**
 * Options for executing a single conversation turn.
 */
export interface PromptOptions {
  /** Optional AbortSignal for cancelling turn execution. */
  signal?: AbortSignal;
  /** Model override for this turn. */
  model?: string;
  /** Sampling temperature override. */
  temperature?: number;
  /** Maximum generation tokens override. */
  maxTokens?: number;
  /** System prompt override. */
  systemPrompt?: string;
  /** Custom tools made available for this turn. */
  tools?: ToolDefinition[];
  /** Real-time event streaming callback. */
  onEvent?: PromptTurnCallback;
}

// ============================================================================
// 12. Streaming Event Callbacks & Event Discriminated Union
// ============================================================================

/**
 * Streaming status event emitted during turn execution.
 */
export interface StatusEvent {
  type: 'status';
  /** Human-readable status message. */
  message: string;
  /** Status level. */
  level?: 'info' | 'warn' | 'error' | 'success';
}

/**
 * Streaming text delta event emitted as LLM response chunks arrive.
 */
export interface TextDeltaEvent {
  type: 'text_delta';
  /** Newly generated text chunk. */
  delta: string;
}

/**
 * Streaming thinking/reasoning delta emitted during model contemplation.
 */
export interface ThinkingDeltaEvent {
  type: 'thinking_delta';
  /** Newly generated reasoning chunk. */
  delta: string;
}

/**
 * Event emitted when a tool call starts executing in the agent workspace.
 */
export interface ToolStartedEvent {
  type: 'tool_started';
  /** Tool call identifier. */
  id: string;
  /** Tool name (e.g. "read", "write", "edit", "grep", "glob", "bash"). */
  name: string;
  /** Tool arguments object. */
  args: Record<string, unknown>;
}

/**
 * Event emitted when a tool call finishes execution.
 */
export interface ToolFinishedEvent {
  type: 'tool_finished';
  /** Tool call identifier matching tool_started. */
  id: string;
  /** Tool name. */
  name: string;
  /** Whether execution succeeded. */
  success: boolean;
  /** Standard output or error message. */
  output: string;
  /** Execution duration in milliseconds. */
  duration_ms: number;
}

/**
 * Event emitted when an advisor agent begins evaluation.
 */
export interface AdvisorStartedEvent {
  type: 'advisor_started';
  /** Advisor identifier (e.g. "Architect", "Security", "Performance"). */
  advisor: string;
  /** Role description. */
  role: string;
}

/**
 * Event emitted when an advisor returns a critique or recommendation.
 */
export interface AdvisorCritiqueEvent {
  type: 'advisor_critique';
  /** Advisor name. */
  advisor: string;
  /** Whether the advisor approved the plan/action. */
  approved: boolean;
  /** Detailed critique or suggestions. */
  critique: string;
  /** Assessed risk level. */
  risk_level?: RiskLevel;
  /** Actionable suggestions. */
  suggestions?: string[];
}

/**
 * Event emitted when a prompt turn completes successfully.
 */
export interface FinishedEvent {
  type: 'finished';
  /** Token usage for this turn. */
  usage: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    cached_tokens?: number;
  };
}

/**
 * Event emitted when an unrecoverable error occurs during turn execution.
 */
export interface ErrorEvent {
  type: 'error';
  /** Error message details. */
  message: string;
  /** Error code if available. */
  code?: number | string;
}

/**
 * Union of all possible streaming events dispatched during `promptTurn`.
 */
export type FusionEvent =
  | StatusEvent
  | TextDeltaEvent
  | ThinkingDeltaEvent
  | ToolStartedEvent
  | ToolFinishedEvent
  | AdvisorStartedEvent
  | AdvisorCritiqueEvent
  | FinishedEvent
  | ErrorEvent;

/**
 * Callback function type receiving streaming events.
 */
export type PromptTurnCallback = (event: FusionEvent) => void;

/**
 * Real-time event emitted during agent turn execution for terminal adapters.
 */
export type AgentEvent = FusionEvent | { type: string; [key: string]: unknown };

/**
 * Callback function for receiving real-time agent streaming events.
 */
export type AgentEventCallback = (event: AgentEvent) => void;

// ============================================================================
// 13. Checkpoints, WebAssembly Bindings & Model Catalog
// ============================================================================

/**
 * Complete snapshot checkpoint of an agent session state.
 */
export interface CheckpointData {
  /** Fusion engine version. */
  version: string;
  /** Serialized session state including messages and token counters. */
  session: {
    id: string;
    active_model: string;
    system_prompt?: string;
    messages: Message[];
    token_stats: TokenStats;
  };
  /** Configuration snapshot. */
  config: FusionConfig;
  /** Virtual filesystem file mapping. */
  vfs: {
    files: Record<string, string>;
  };
  /** Number of executed conversation turns. */
  turn_counter: number;
}

/**
 * Low-level WebAssembly instance bindings exported by wasm-bindgen.
 */
export interface WasmFusionAgentBindings {
  get_session_id(): string;
  get_active_model(): string;
  set_active_model(model: string): void;
  set_system_prompt(prompt: string): void;
  get_messages(): string;
  get_token_stats(): string;
  clear_messages(): void;
  fs_write(path: string, content: string): void;
  fs_read(path: string): string;
  fs_list(): string;
  fs_delete(path: string): boolean;
  checkpoint(): string;
  restore(checkpoint_json: string): void;
  prompt_turn(input_str: string, callback?: (event: unknown) => void): Promise<string>;
}

/**
 * WASM Module Initialization Options.
 */
export interface WasmInitOptions {
  /** Optional custom URL or Path to fusion.wasm. */
  wasmUrl?: string | URL;
  /** Optional pre-loaded WebAssembly.Module or ArrayBuffer / Response. */
  wasmBinary?: ArrayBuffer | Uint8Array | Response | WebAssembly.Module;
}

/**
 * Model Catalog Entry for model picker UI components.
 */
export interface ModelCatalogEntry {
  /** Model ID (e.g. "anthropic/claude-3.5-sonnet"). */
  id: string;
  /** Display name. */
  name: string;
  /** Provider identifier. */
  provider: string;
  /** Functional category. */
  category: 'coding' | 'reasoning' | 'fast' | 'general';
  /** Optional feature badge/tag (e.g. "Recommended", "Fast", "Top Tier"). */
  tag?: string;
  /** Context window description. */
  context: string;
  /** Pricing information string. */
  pricing: string;
  /** Model capability overview. */
  description: string;
}

/**
 * Transport abstraction for communicating with remote or local Fusion agents.
 */
export interface AgentTransport {
  /** Transport medium identifier. */
  readonly type: 'wasm' | 'websocket' | 'stdio' | 'http' | 'custom';
  /** Connection endpoint URL or path (if applicable). */
  readonly endpoint?: string;
  /** Whether the transport is currently connected. */
  readonly isConnected: boolean;
  /** Connect to the agent backend. */
  connect(): Promise<void>;
  /** Send a raw or JSON-RPC message. */
  send(message: string | JsonRpcRequest): Promise<void>;
  /** Register a message listener. */
  onMessage(handler: (data: string | JsonRpcResponse | JsonRpcNotification) => void): () => void;
  /** Disconnect and clean up resources. */
  disconnect(): Promise<void> | void;
}
