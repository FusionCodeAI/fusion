import { describe, test, expect, beforeEach } from "bun:test";
import {
  FUSION_MODELS,
  getAllAvailableModels,
  addCustomModel,
  removeCustomModel,
  getFusionModel,
  getModelById,
} from "../src/models";
describe("Dynamic Models Architecture (Cline Pattern)", () => {
  test("includes built-in models from multiple providers (Fusion, Anthropic, OpenAI, DeepSeek, Ollama)", () => {
    const models = getAllAvailableModels();
    expect(models.length).toBeGreaterThanOrEqual(12);

    const providers = new Set(models.map((m) => m.provider));
    expect(providers.has("fusion")).toBe(true);
    expect(providers.has("anthropic")).toBe(true);
    expect(providers.has("openai")).toBe(true);
    expect(providers.has("deepseek")).toBe(true);
    expect(providers.has("ollama")).toBe(true);
  });

  test("adds custom model ID dynamically and makes it available", () => {
    const customId = "meta-llama/llama-3.3-70b-instruct";
    const custom = addCustomModel(customId, "Llama 3.3 70B");

    expect(custom.id).toBe(customId);
    expect(custom.isCustom).toBe(true);
    expect(custom.provider).toBe("custom");

    const found = getModelById(customId);
    expect(found).toBeDefined();
    expect(found?.name).toBe("Llama 3.3 70B");

    removeCustomModel(customId);
    const afterRemove = getModelById(customId);
    expect(afterRemove).toBeUndefined();
  });

  test("getFusionModel falls back to default model for unknown model ID strings", () => {
    const arbitrary = "some-brand-new-frontier-model-v5";
    const model = getFusionModel(arbitrary);

    expect(model.id).toBe("deepseek-v4-flash-0731");
  });
});
