import { defaultConnection, type ProviderSettings } from "../../../src/lib/smartCheckProviders"

export function fixtureProviderSettings(): ProviderSettings {
  const jev = defaultConnection("jev")
  const ollama = { ...defaultConnection("ollama"), model: "synthetic-local-model" }
  const cloudflare = defaultConnection("cloudflare")
  cloudflare.endpoint = { kind: "cloudflare_account", value: "synthetic-account" }
  const custom = {
    ...defaultConnection("custom"),
    model: "synthetic-custom-model",
    endpoint: { kind: "exact_url" as const, value: "https://example.invalid/systemone" },
    contextOverride: {
      totalInputTokens: 8192,
      stateAndLongestQuestionTokens: null,
      runtimeContextTokens: null,
    },
  }
  return { activeId: "jev", profiles: { jev, ollama, cloudflare, custom } }
}
