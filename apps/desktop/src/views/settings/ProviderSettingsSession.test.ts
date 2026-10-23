import { beforeEach, expect, it, vi } from "vitest"
import {
  defaultConnection,
  type ConnectionDraft,
  type ProviderSettings,
} from "../../lib/smartCheckProviders"
import { ProviderSettingsSession, providerErrorMessage } from "./ProviderSettingsSession"

const invoke = vi.hoisted(() => vi.fn())
const removeLegacyKey = vi.hoisted(() => vi.fn())
const confirmRemoval = vi.hoisted(() => vi.fn())
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
vi.mock("../../lib/checkAvailability", () => ({ removeTypeSafeApiKey: removeLegacyKey }))
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: confirmRemoval }))

let saved: ProviderSettings
beforeEach(() => {
  saved = {
    activeId: "jev",
    profiles: {
      jev: defaultConnection("jev"),
      local: { ...defaultConnection("ollama"), model: "synthetic-local" },
    },
  }
  removeLegacyKey.mockReset().mockResolvedValue(undefined)
  confirmRemoval.mockReset().mockResolvedValue(true)
  invoke.mockReset().mockImplementation(async (command) => {
    if (command === "get_system_one_settings") return saved
    throw new Error(`Unexpected command: ${command}`)
  })
})

it.each([
  "The selected provider does not support this model or runner.",
  "The selected provider did not report usable model limits.",
  "Could not read model limits from the selected provider. Check the connection and try again.",
  "A connection update is incomplete. Retry it in Settings → Checks before removing a credential.",
  "Could not cancel unfinished Burn Checks. Checks are paused. Retry the connection change.",
  "The connection revision limit was reached. Save a new connection.",
])("preserves the native Settings instruction: %s", (message) => {
  expect(providerErrorMessage(new Error(message), "fallback")).toBe(message)
})

it("refreshes saved state without losing dirty drafts or making provider or analytics calls", async () => {
  const session = new ProviderSettingsSession()
  await session.refresh()
  session.edit(defaultConnection("jev"), "synthetic-draft")
  saved = {
    activeId: "local",
    profiles: {
      ...saved.profiles,
      local: { ...saved.profiles.local!, model: "updated-model" },
    },
  }
  await session.refresh()
  expect(session.getSnapshot().saved?.activeId).toBe("local")
  expect(session.getSnapshot().selectedId).toBe("jev")
  expect(session.getSnapshot().drafts.jev!.draft.credential).toBe("synthetic-draft")
  expect(session.getSnapshot().drafts.local!.draft.connection.model).toBe("updated-model")
  expect(invoke.mock.calls.map(([command]) => command)).toEqual([
    "get_system_one_settings",
    "get_system_one_settings",
  ])
})

it("does not send a no-op switch or activate an unsaved draft", async () => {
  const session = new ProviderSettingsSession()
  await session.refresh()
  await session.run("switch")
  session.select("local")
  session.edit({ ...saved.profiles.local!, model: "draft-model" }, null)
  await session.run("switch")
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["get_system_one_settings"])
  expect(session.getSnapshot().saved?.activeId).toBe("jev")
})

it("keeps a failed switch on the saved active profile and preserves other drafts", async () => {
  const session = new ProviderSettingsSession()
  await session.refresh()
  session.edit(defaultConnection("jev"), "keep-draft")
  session.select("local")
  invoke.mockRejectedValueOnce(new Error("Could not complete the connection update."))
  await session.run("switch")
  expect(session.getSnapshot().saved?.activeId).toBe("jev")
  expect(session.getSnapshot().error).toBe("Could not complete the connection update.")
  expect(session.getSnapshot().drafts.jev!.draft.credential).toBe("keep-draft")
})

it("keeps saved references and replacement input after an incomplete credential removal", async () => {
  saved.profiles.local = {
    ...saved.profiles.local!,
    credential: { kind: "connection", id: "local-key" },
  }
  const session = new ProviderSettingsSession()
  await session.refresh()
  session.select("local")
  session.edit(saved.profiles.local, "synthetic-replacement")
  invoke.mockRejectedValueOnce(
    new Error("Connection credential removal is incomplete. Retry it in Settings → Checks."),
  )
  await session.run("remove")
  expect(session.getSnapshot().error).toContain("credential removal is incomplete")
  expect(session.getSnapshot().drafts.local!.draft.credential).toBe("synthetic-replacement")
  expect(session.getSnapshot().saved?.profiles.local!.credential).toEqual({
    kind: "connection",
    id: "local-key",
  })
  expect(session.getSnapshot().busy).toBe(false)
})

it("removes the legacy Jev key through its existing native API", async () => {
  saved.profiles.jev = { ...defaultConnection("jev"), credential: { kind: "legacy_type_safe" } }
  const session = new ProviderSettingsSession()
  await session.refresh()
  removeLegacyKey.mockImplementationOnce(async () => {
    saved = { ...saved, profiles: { ...saved.profiles, jev: defaultConnection("jev") } }
  })
  await session.run("remove")
  expect(removeLegacyKey).toHaveBeenCalledOnce()
  expect(session.getSnapshot().drafts.jev!.draft.connection.credential).toBeNull()
  expect(invoke.mock.calls.map(([command]) => command)).toEqual([
    "get_system_one_settings",
    "get_system_one_settings",
  ])
})

it("keeps a failed limit refresh unavailable and does not substitute manual overrides", async () => {
  const session = new ProviderSettingsSession()
  await session.refresh()
  session.select("local")
  session.edit(
    {
      ...saved.profiles.local!,
      contextOverride: {
        totalInputTokens: 8192,
        stateAndLongestQuestionTokens: null,
        runtimeContextTokens: null,
      },
    },
    null,
  )
  invoke.mockRejectedValueOnce(new Error("Ollama rejected the configured authentication"))
  await session.run("refresh")
  expect(session.getSnapshot().error).toBe("Ollama rejected the configured authentication")
  expect(session.getSnapshot().drafts.local!.capabilities).toBeNull()
  expect(session.getSnapshot().saved?.activeId).toBe("jev")
})

it("defers search navigation until a pending save completes without adding provider calls", async () => {
  const session = new ProviderSettingsSession()
  await session.refresh()
  session.edit(defaultConnection("jev"), "synthetic-draft")
  let resolve!: (saved: ProviderSettings) => void
  invoke.mockImplementationOnce(
    (_command, { draft }: { draft: ConnectionDraft }) =>
      new Promise<ProviderSettings>((done) => {
        resolve = () =>
          done({
            ...saved,
            profiles: {
              ...saved.profiles,
              jev: { ...draft.connection, credential: { kind: "connection", id: "jev-key" } },
            },
          })
      }),
  )
  const saving = session.run("save")
  session.revealTarget("ollamaUrl", 1)
  expect(session.getSnapshot().selectedId).toBe("jev")
  resolve(saved)
  await saving
  expect(session.getSnapshot().selectedId).toBe("local")
  expect(session.getSnapshot().saved?.activeId).toBe("jev")
  expect(invoke.mock.calls.map(([command]) => command)).toEqual([
    "get_system_one_settings",
    "save_system_one_connection",
  ])
})

it("does not let a late read replace a newer snapshot", async () => {
  const session = new ProviderSettingsSession()
  let resolve!: (settings: ProviderSettings) => void
  invoke.mockImplementationOnce(
    () =>
      new Promise<ProviderSettings>((done) => {
        resolve = done
      }),
  )
  const first = session.refresh()
  const current = { ...saved, activeId: "local" }
  invoke.mockResolvedValueOnce(current)
  await session.refresh()
  resolve(saved)
  await first
  expect(session.getSnapshot().saved?.activeId).toBe("local")
})
