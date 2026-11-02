// Which supplier a model id belongs to, read from its prefix (`direct/openai/gpt-5` is OpenAI's,
// `local/llama3.1` this computer's, a bare `vendor/model` OpenRouter's).

import { SUPPLIERS, type Supplier } from "./constants";

/** OpenRouter: models with no supplier prefix go there. */
const MAIN: Supplier = SUPPLIERS.find((s) => s.prefix === "") ?? { id: "openrouter", name: "OpenRouter", prefix: "", about: "" };

/** The supplier `model` goes to: the one with the longest prefix it starts with. */
export function supplierOf(model: string): Supplier {
  let best: Supplier | undefined;
  for (const s of SUPPLIERS) {
    if (s.prefix && model.startsWith(s.prefix) && s.prefix.length > (best?.prefix.length ?? 0)) best = s;
  }
  return best ?? MAIN;
}

/** `model` without its supplier's prefix: what the supplier itself calls it. */
export function bareModel(model: string): string {
  return model.slice(supplierOf(model).prefix.length);
}

/** The supplier called `id`. */
export function supplier(id: string): Supplier | undefined {
  return SUPPLIERS.find((s) => s.id === id);
}
