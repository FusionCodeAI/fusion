/**
 * Canonical Fusion AI models catalog matching `src/provider/catalog.rs`.
 */

export interface FusionModel {
  id: string;
  name: string;
  shortName: string;
  badge: string;
  contextWindow: string;
  description: string;
}

export const FUSION_MODELS: FusionModel[] = [
  {
    id: "deepseek-v4-flash-0731",
    name: "DeepSeek 4 0731 Flash",
    shortName: "DeepSeek 4 Flash",
    badge: "Default · 1M",
    contextWindow: "1M context",
    description: "Fusion high-speed 1M context model",
  },
  {
    id: "deepseek-v4-flash-0731-fast",
    name: "DeepSeek 4 0731 Flash Fast",
    shortName: "DeepSeek 4 Fast",
    badge: "Speed",
    contextWindow: "1M context",
    description: "Fusion ultra-low latency model",
  },
  {
    id: "glm-5.3-flash",
    name: "GLM 5.3 Flash",
    shortName: "GLM 5.3 Flash",
    badge: "1M Context",
    contextWindow: "1M context",
    description: "Fusion GLM 5.3 Flash 1M model",
  },
  {
    id: "minimax-m2.7",
    name: "MiniMax M2.7",
    shortName: "MiniMax M2.7",
    badge: "Reasoning",
    contextWindow: "200K context",
    description: "MiniMax M2.7 frontier coding and reasoning model",
  },
];

export const DEFAULT_FUSION_MODEL = FUSION_MODELS[0];

/**
 * Look up a Fusion model by exact ID.
 */
export function getModelById(id: string): FusionModel | undefined {
  return FUSION_MODELS.find((m) => m.id === id);
}

/**
 * Look up a Fusion model by ID or shortName, falling back to default model.
 */
export function getFusionModel(idOrName?: string): FusionModel {
  if (!idOrName) return DEFAULT_FUSION_MODEL;
  const found = FUSION_MODELS.find(
    (m) => m.id === idOrName || m.shortName.toLowerCase() === idOrName.toLowerCase()
  );
  return found ?? DEFAULT_FUSION_MODEL;
}

/**
 * Check if a string is a valid canonical Fusion model ID.
 */
export function isValidModelId(id: string): boolean {
  return FUSION_MODELS.some((m) => m.id === id);
}
