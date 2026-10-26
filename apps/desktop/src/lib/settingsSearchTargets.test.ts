import { expect, it } from "vitest"
import { resolveSettingsSearchTarget, searchApp } from "./appSearch"
import { settingsControlLabel } from "./settingsSearchTargets"

it.each(["macos", "windows", "linux"] as const)(
  "keeps model-based check search destinations on %s",
  (platform) => {
    expect(settingsControlLabel("smartChecksEnabled", platform)).toBe("Run model-based checks")
    for (const [query, control] of [
      ["Run model-based checks", "smartChecksEnabled"],
      ["Model-based checks", "smartChecksEnabled"],
      ["discovery", "smartChecksEnabled"],
      ["TypeSafe API key", "typeSafeApiKey"],
      ["Jev API key", "typeSafeApiKey"],
      ["decision model", "decisionProvider"],
      ["saved connections", "decisionConnection"],
      ["Ollama URL", "ollamaUrl"],
      ["Ollama model", "ollamaModel"],
      ["Ollama API key", "ollamaApiKey"],
      ["Cloudflare account ID", "cloudflareAccountId"],
      ["Cloudflare API token", "cloudflareApiToken"],
      ["clef-flash", "cloudflareModel"],
      ["Custom endpoint", "customUrl"],
      ["Custom model", "customModel"],
      ["Custom API key", "customApiKey"],
      ["response mode", "customResponseMode"],
      ["Model limits", "decisionModelLimits"],
      ["Total input tokens", "decisionTotalInput"],
      ["State and longest question tokens", "decisionStateInput"],
      ["Loaded context tokens", "decisionRuntimeContext"],
    ] as const) {
      const result = searchApp(query, platform).find(
        (result) => result.target.kind === "setting" && result.target.control === control,
      )
      if (!result || result.target.kind !== "setting")
        throw new Error(`Missing search destination for ${query}`)
      expect(resolveSettingsSearchTarget(result.target)).toEqual({
        pane: "checks",
        control,
      })
    }
  },
)
