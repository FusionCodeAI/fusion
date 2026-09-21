export type MessageRole = "user" | "assistant" | "system";

export interface ChatImageAttachment {
  id: string;
  url: string; // Base64 data URL or local image path
  name?: string;
  size?: number;
}

export interface TurnStep {
  id: string;
  title: string;
  status: "running" | "completed" | "failed";
  timestamp?: number;
  details?: string;
}

export interface ChatMessage {
  id: string;
  role: MessageRole;
  content: string;
  thought?: string;
  steps?: TurnStep[];
  diffPatch?: string;
  images?: ChatImageAttachment[];
  timestamp: number;
}

export interface ChatSession {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  workspaceDir: string;
  model: string;
  messages: ChatMessage[];
}

export interface AcpEvent {
  method: string;
  params: Record<string, unknown>;
}
