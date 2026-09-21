/**
 * Canonical Fusion AI models catalog matching `src/provider/catalog.rs` and Cline architecture.
 */

export type ModelProvider =
  | "fusion"
  | "anthropic"
  | "openai"
  | "deepseek"
  | "openrouter"
  | "ollama"
  | "custom";

export interface FusionModel {
  id: string;
  name: string;
  shortName: string;
  provider: ModelProvider;
  badge: string;
  contextWindow: string;
  description: string;
  supportsReasoning?: boolean;
  isCustom?: boolean;
}

/**
 * The 4 canonical Fusion models matching `src/provider/catalog.rs` defaults.
 */
export const FUSION_MODELS: FusionModel[] = [
  {
    id: "deepseek-v4-flash-0731",
    name: "DeepSeek 4 0731 Flash",
    shortName: "DeepSeek 4 Flash",
    provider: "fusion",
    badge: "Default · 1M",
    contextWindow: "1M context",
    description: "Fusion high-speed 1M context flagship model",
    supportsReasoning: false,
  },
  {
    id: "deepseek-v4-flash-0731-fast",
    name: "DeepSeek 4 0731 Flash Fast",
    shortName: "DeepSeek 4 Fast",
    provider: "fusion",
    badge: "Speed",
    contextWindow: "1M context",
    description: "Fusion ultra-low latency model",
    supportsReasoning: false,
  },
  {
    id: "glm-5.3-flash",
    name: "GLM 5.3 Flash",
    shortName: "GLM 5.3 Flash",
    provider: "fusion",
    badge: "1M Context",
    contextWindow: "1M context",
    description: "Fusion GLM 5.3 Flash 1M model",
    supportsReasoning: false,
  },
  {
    id: "minimax-m2.7",
    name: "MiniMax M2.7",
    shortName: "MiniMax M2.7",
    provider: "fusion",
    badge: "Reasoning",
    contextWindow: "200K context",
    description: "MiniMax M2.7 frontier coding and reasoning model",
    supportsReasoning: true,
  },
];

export const DEFAULT_FUSION_MODEL = FUSION_MODELS[0];

/**
 * Extended catalog of supported multi-provider models matching Cline & Fusion provider catalogs.
 */
export const EXTENDED_PROVIDER_MODELS: FusionModel[] = [
  // Anthropic Models
  {
    id: "claude-3-7-sonnet-20250219",
    name: "Claude 3.7 Sonnet",
    shortName: "Claude 3.7 Sonnet",
    provider: "anthropic",
    badge: "Thinking · 200K",
    contextWindow: "200K context",
    description: "Anthropic hybrid reasoning & coding model",
    supportsReasoning: true,
  },
  {
    id: "claude-3-5-sonnet-20241022",
    name: "Claude 3.5 Sonnet",
    shortName: "Claude 3.5 Sonnet",
    provider: "anthropic",
    badge: "200K",
    contextWindow: "200K context",
    description: "Anthropic flagship coding intelligence model",
    supportsReasoning: false,
  },
  {
    id: "claude-3-5-haiku-20241022",
    name: "Claude 3.5 Haiku",
    shortName: "Claude 3.5 Haiku",
    provider: "anthropic",
    badge: "Fast · 200K",
    contextWindow: "200K context",
    description: "Anthropic lightweight rapid coding model",
    supportsReasoning: false,
  },

  // OpenAI Models
  {
    id: "o3-mini",
    name: "o3-mini",
    shortName: "o3-mini",
    provider: "openai",
    badge: "Reasoning · 200K",
    contextWindow: "200K context",
    description: "OpenAI fast STEM and competitive coding reasoning model",
    supportsReasoning: true,
  },
  {
    id: "o1",
    name: "o1",
    shortName: "o1",
    provider: "openai",
    badge: "Reasoning · 200K",
    contextWindow: "200K context",
    description: "OpenAI deep frontier reasoning model",
    supportsReasoning: true,
  },
  {
    id: "gpt-4o",
    name: "GPT-4o",
    shortName: "GPT-4o",
    provider: "openai",
    badge: "Omni · 128K",
    contextWindow: "128K context",
    description: "OpenAI high-intelligence flagship model",
    supportsReasoning: false,
  },

  // DeepSeek Native Models
  {
    id: "deepseek-reasoner",
    name: "DeepSeek R1",
    shortName: "DeepSeek R1",
    provider: "deepseek",
    badge: "Reasoning · 64K",
    contextWindow: "64K context",
    description: "DeepSeek open weights reasoning & math model",
    supportsReasoning: true,
  },
  {
    id: "deepseek-chat",
    name: "DeepSeek V3",
    shortName: "DeepSeek V3",
    provider: "deepseek",
    badge: "64K",
    contextWindow: "64K context",
    description: "DeepSeek general chat & coding model",
    supportsReasoning: false,
  },

  // Local Ollama Models
  {
    id: "qwen2.5-coder:32b",
    name: "Qwen 2.5 Coder 32B",
    shortName: "Qwen 2.5 Coder",
    provider: "ollama",
    badge: "Local · 32K",
    contextWindow: "32K context",
    description: "Alibaba top open-source coding model via Ollama",
    supportsReasoning: false,
  },
];

const CUSTOM_MODELS_KEY = "fusion_desktop_custom_models_v1";

let customModelsMemory: FusionModel[] = [];

export function clearCustomModelsMemory(): void {
  customModelsMemory = [];
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.removeItem(CUSTOM_MODELS_KEY);
  }
}

/**
 * Loads user-defined custom models from storage.
 */
export function loadCustomModels(): FusionModel[] {
  if (typeof window === "undefined" || !window.localStorage) {
    return customModelsMemory;
  }
  try {
    const raw = window.localStorage.getItem(CUSTOM_MODELS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed;
  } catch {
    return [];
  }
}

/**
 * Persists a user-defined custom model.
 */
export function addCustomModel(modelId: string, name?: string): FusionModel {
  const trimmed = modelId.trim();
  const cleanName = (name || trimmed).trim();
  const customModel: FusionModel = {
    id: trimmed,
    name: cleanName,
    shortName: cleanName.length > 24 ? `${cleanName.slice(0, 22)}...` : cleanName,
    provider: "custom",
    badge: "Custom",
    contextWindow: "Custom",
    description: `Custom model: ${trimmed}`,
    supportsReasoning:
      trimmed.includes("r1") ||
      trimmed.includes("o1") ||
      trimmed.includes("o3") ||
      trimmed.includes("thinking") ||
      trimmed.includes("reason"),
    isCustom: true,
  };

  const existing = loadCustomModels();
  const filtered = existing.filter((m) => m.id !== trimmed);
  const updated = [...filtered, customModel];

  customModelsMemory = updated;
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.setItem(CUSTOM_MODELS_KEY, JSON.stringify(updated));
  }

  return customModel;
}

/**
 * Removes a custom model from storage.
 */
export function removeCustomModel(modelId: string): void {
  const existing = loadCustomModels();
  const updated = existing.filter((m) => m.id !== modelId);
  customModelsMemory = updated;
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.setItem(CUSTOM_MODELS_KEY, JSON.stringify(updated));
  }
}

/**
 * Returns all available models (built-in 4 canonical + extended provider catalog + custom models).
 */
export function getAllAvailableModels(): FusionModel[] {
  const custom = loadCustomModels();
  return [...FUSION_MODELS, ...EXTENDED_PROVIDER_MODELS, ...custom];
}

/**
 * Look up a model by exact ID across built-in and custom models.
 */
export function getModelById(id: string): FusionModel | undefined {
  return getAllAvailableModels().find((m) => m.id === id);
}

/**
 * Look up a Fusion model by ID, alias, or shortName, falling back to default model.
 */
export function getFusionModel(idOrName?: string): FusionModel {
  if (!idOrName) return DEFAULT_FUSION_MODEL;
  const lower = idOrName.toLowerCase();

  // Handle common shorthand aliases
  if (lower === "deepseek-4-flash" || lower === "deepseek-flash") {
    return FUSION_MODELS[0];
  }
  if (lower === "deepseek-4-fast" || lower === "deepseek-fast") {
    return FUSION_MODELS[1];
  }

  const all = getAllAvailableModels();
  const found = all.find(
    (m) =>
      m.id === idOrName ||
      m.shortName.toLowerCase() === lower ||
      m.name.toLowerCase() === lower
  );

  return found ?? DEFAULT_FUSION_MODEL;
}

/**
 * Checks if a string is one of the 4 canonical Fusion model IDs.
 */
export function isValidModelId(id: string): boolean {
  return FUSION_MODELS.some((m) => m.id === id);
}

/**
 * Checks if a string is any supported model ID (canonical, provider, or custom).
 */
export function isSupportedModelId(id: string): boolean {
  return getAllAvailableModels().some((m) => m.id === id);
}
