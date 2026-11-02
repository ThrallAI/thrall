import { describe, expect, it } from "vitest";

import { bareModel, supplierOf } from "./suppliers";

describe("supplierOf", () => {
  it("reads the supplier from the model id's prefix", () => {
    expect(supplierOf("direct/openai/gpt-5").id).toBe("openai");
    expect(supplierOf("direct/elevenlabs/eleven_multilingual_v2").id).toBe("elevenlabs");
    expect(supplierOf("local/llama3.1").id).toBe("local");
    expect(supplierOf("chatgpt/gpt-5.3-codex").id).toBe("chatgpt");
  });

  it("gives OpenRouter everything else, its own vendor names included", () => {
    expect(supplierOf("openai/gpt-5").id).toBe("openrouter");
    expect(supplierOf("xiaomi/mimo-v2.5-pro").id).toBe("openrouter");
    expect(supplierOf("").id).toBe("openrouter");
  });

  it("drops the prefix for the supplier's own name", () => {
    expect(bareModel("direct/anthropic/sonnet-4-5")).toBe("sonnet-4-5");
    expect(bareModel("google/gemini-2.5-flash")).toBe("google/gemini-2.5-flash");
  });
});
