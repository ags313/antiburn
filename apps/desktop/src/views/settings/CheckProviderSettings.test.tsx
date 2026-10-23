import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { beforeEach, expect, it, vi } from "vitest"

import {
  defaultConnection,
  type Connection,
  type ModelCapabilities,
  type ProviderSettings,
  type SmartCheckProvider,
} from "../../lib/smartCheckProviders"
import { searchApp } from "../../lib/appSearch"
import { CheckProviderSettings } from "./CheckProviderSettings"

const invoke = vi.hoisted(() => vi.fn())
const confirmRemoval = vi.hoisted(() => vi.fn())
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: confirmRemoval }))

let saved: ProviderSettings
const limit = { value: 8192, source: "manual" as const }
const capabilities: ModelCapabilities = {
  output: {
    choice: {
      probabilities: true,
      confidence: true,
      minimum_criteria: 2,
      maximum_criteria: 26,
    },
    score: null,
    noul: false,
  },
  total_input_tokens: limit,
  state_and_longest_question_tokens: limit,
  runtime_context_tokens: limit,
  request_body_bytes: { value: null, source: "unknown" },
  questions_per_request: { value: 1, source: "documented_default" },
  rendering_reserve_tokens: 4096,
  tokenizer: null,
  model: "synthetic-model",
  model_revision: null,
}

function configured(provider: SmartCheckProvider): Connection {
  const connection = {
    ...defaultConnection(provider),
    model:
      provider === "jev"
        ? defaultConnection("jev").model
        : provider === "cloudflare"
          ? "clef"
          : "synthetic-model",
  }
  if (connection.endpoint.kind !== "provider_default")
    connection.endpoint = {
      ...connection.endpoint,
      value:
        provider === "cloudflare" ? "synthetic-account" : "http://localhost:11434/inference",
    }
  if (provider === "jev" || provider === "cloudflare")
    connection.credential = { kind: "connection", id: `${provider}-key` }
  if (provider === "custom")
    connection.contextOverride = {
      totalInputTokens: 8192,
      stateAndLongestQuestionTokens: null,
      runtimeContextTokens: null,
    }
  return connection
}

beforeEach(() => {
  saved = { activeId: "jev", profiles: { jev: defaultConnection("jev") } }
  confirmRemoval.mockReset().mockResolvedValue(true)
  invoke.mockReset()
  invoke.mockImplementation(async (command, args) => {
    if (command === "get_system_one_settings") return saved
    if (command === "test_system_one_connection") return null
    if (command === "refresh_system_one_limits") return capabilities
    if (command === "save_system_one_connection") {
      const connection = {
        ...args.draft.connection,
        credential: args.draft.credential
          ? { kind: "connection", id: args.draft.connection_id }
          : args.draft.connection.credential,
      }
      saved = {
        activeId: args.draft.connection_id,
        profiles: { ...saved.profiles, [args.draft.connection_id]: connection },
      }
      return saved
    }
    if (command === "switch_system_one_connection") {
      saved = { ...saved, activeId: args.connectionId }
      return saved
    }
    if (command === "remove_system_one_credential") {
      saved = {
        ...saved,
        profiles: {
          ...saved.profiles,
          [args.connectionId]: { ...saved.profiles[args.connectionId]!, credential: null },
        },
      }
      return null
    }
    if (command === "remove_typesafe_api_key") {
      saved = {
        ...saved,
        profiles: Object.fromEntries(
          Object.entries(saved.profiles).map(([id, profile]) => [
            id,
            profile.credential?.kind === "legacy_type_safe"
              ? { ...profile, credential: null }
              : profile,
          ]),
        ),
      }
      return null
    }
    throw new Error(`Unexpected command: ${command}`)
  })
})

it.each(["jev", "ollama", "cloudflare", "custom"] as const)(
  "restores the active %s profile without tests, saves, or provider calls",
  async (provider) => {
    saved = {
      activeId: "selected-profile",
      profiles: { jev: defaultConnection("jev"), "selected-profile": configured(provider) },
    }
    render(<CheckProviderSettings />)
    await waitFor(() => expect(screen.getByLabelText("Provider")).toHaveValue(provider))
    expect(screen.getByLabelText("Saved connection")).toHaveValue("selected-profile")
    expect(screen.getByText(/^Active:/)).toHaveTextContent(
      saved.profiles["selected-profile"]!.model,
    )
    expect(screen.getByRole("button", { name: "Model limits" })).toHaveAttribute(
      "aria-expanded",
      "false",
    )
    expect(screen.getByRole("button", { name: "Save and use" })).toBeEnabled()
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(["get_system_one_settings"])
  },
)

it.each([
  ["jev", null],
  ["ollama", null],
  ["cloudflare", null],
  ["custom", "direct"],
  ["custom", "cloudflare_envelope"],
] as const)(
  "tests and saves %s %s only after an explicit action",
  async (provider, responseMode) => {
    const connection = configured(provider)
    if (responseMode) connection.responseMode = responseMode
    saved.profiles.profile = connection
    render(<CheckProviderSettings />)
    await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
    fireEvent.change(screen.getByLabelText("Saved connection"), {
      target: { value: "profile" },
    })
    const keyLabel =
      provider === "jev"
        ? "TypeSafe API key"
        : provider === "cloudflare"
          ? "Cloudflare API token"
          : provider === "ollama"
            ? "Ollama API key (optional)"
            : "Custom API key (optional)"
    fireEvent.change(screen.getByLabelText(keyLabel), {
      target: { value: "synthetic-replacement" },
    })
    expect(saved.activeId).toBe("jev")
    expect(screen.getByRole("button", { name: "Use connection" })).toBeDisabled()
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }))
    await screen.findByText("Connection test passed.")
    expect(saved.activeId).toBe("jev")
    expect(invoke).toHaveBeenCalledWith("test_system_one_connection", {
      draft: { connection_id: "profile", connection, credential: "synthetic-replacement" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save and use" }))
    await screen.findByText("Connection saved and active.")
    expect(saved.activeId).toBe("profile")
    expect(screen.getByLabelText(keyLabel)).toHaveValue("")
    expect(saved.profiles.profile!.responseMode).toBe(responseMode ?? connection.responseMode)
  },
)

it("keeps unsaved drafts and credentials separate between providers and saved profiles", async () => {
  saved.profiles.localA = configured("ollama")
  saved.profiles.localB = { ...configured("ollama"), model: "other-model" }
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
  fireEvent.change(screen.getByLabelText("TypeSafe API key"), {
    target: { value: "synthetic-jev" },
  })
  fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "ollama" } })
  fireEvent.change(screen.getByLabelText("Ollama model"), { target: { value: "draft-model" } })
  fireEvent.change(screen.getByLabelText("Ollama API key (optional)"), {
    target: { value: "synthetic-ollama" },
  })
  fireEvent.change(screen.getByLabelText("Saved connection"), { target: { value: "localB" } })
  expect(screen.getByLabelText("Ollama model")).toHaveValue("other-model")
  expect(screen.getByLabelText("Ollama API key (optional)")).toHaveValue("")
  fireEvent.change(screen.getByLabelText("Saved connection"), { target: { value: "localA" } })
  expect(screen.getByLabelText("Ollama model")).toHaveValue("draft-model")
  expect(screen.getByLabelText("Ollama API key (optional)")).toHaveValue("synthetic-ollama")
  fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "jev" } })
  expect(screen.getByLabelText("TypeSafe API key")).toHaveValue("synthetic-jev")
  expect(saved.activeId).toBe("jev")
  expect(saved.profiles.localA!.model).toBe("synthetic-model")
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["get_system_one_settings"])
})

it.each([
  ["jev", "direct"],
  ["ollama", "direct"],
  ["cloudflare", "cloudflare_envelope"],
  ["custom", "direct"],
  ["custom", "cloudflare_envelope"],
] as const)(
  "uses an unchanged saved %s %s connection without saving a draft",
  async (provider, responseMode) => {
    saved.profiles.target = { ...configured(provider), responseMode }
    render(<CheckProviderSettings />)
    await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
    fireEvent.change(screen.getByLabelText("Saved connection"), { target: { value: "target" } })
    expect(saved.activeId).toBe("jev")
    expect(screen.getByRole("button", { name: "Save and use" })).toBeEnabled()
    fireEvent.click(screen.getByRole("button", { name: "Use connection" }))
    await screen.findByText("Connection active.")
    expect(saved.activeId).toBe("target")
    expect(invoke).toHaveBeenCalledWith("switch_system_one_connection", {
      connectionId: "target",
    })
    expect(invoke).not.toHaveBeenCalledWith("save_system_one_connection", expect.anything())
  },
)

it.each(["ollama", "custom"] as const)(
  "accepts an optional empty %s credential",
  async (provider) => {
    saved = { activeId: "target", profiles: { target: configured(provider) } }
    render(<CheckProviderSettings />)
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Test connection" })).toBeEnabled(),
    )
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }))
    await screen.findByText("Connection test passed.")
    expect(invoke).toHaveBeenCalledWith(
      "test_system_one_connection",
      expect.objectContaining({ draft: expect.objectContaining({ credential: null }) }),
    )
    fireEvent.change(
      screen.getByLabelText(provider === "ollama" ? "Ollama model" : "Custom model"),
      { target: { value: "edited-model" } },
    )
    fireEvent.click(screen.getByRole("button", { name: "Save and use" }))
    await screen.findByText("Connection saved and active.")
    expect(saved.profiles.target!.credential).toBeNull()
    expect(saved.profiles.target!.model).toBe("edited-model")
  },
)

it("shows only the selected provider fields and supports Cloudflare models and Custom modes", async () => {
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
  expect(screen.queryByLabelText("Ollama model")).not.toBeInTheDocument()
  fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "cloudflare" } })
  fireEvent.change(screen.getByLabelText("Cloudflare account ID"), {
    target: { value: "synthetic-account" },
  })
  fireEvent.change(screen.getByLabelText("Cloudflare API token"), {
    target: { value: "synthetic-token" },
  })
  fireEvent.change(screen.getByLabelText("Cloudflare model"), {
    target: { value: "clef-flash" },
  })
  fireEvent.click(screen.getByRole("button", { name: "Save and use" }))
  await screen.findByText("Connection saved and active.")
  expect(saved.profiles.cloudflare!.model).toBe("clef-flash")
  fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "custom" } })
  fireEvent.change(screen.getByLabelText("Exact inference URL"), {
    target: { value: "https://example.invalid/exact/path?route=decision" },
  })
  fireEvent.change(screen.getByLabelText("Custom model"), {
    target: { value: "synthetic-custom" },
  })
  fireEvent.change(screen.getByLabelText("Response mode"), {
    target: { value: "cloudflare_envelope" },
  })
  expect(screen.getByRole("button", { name: "Save and use" })).toBeDisabled()
  fireEvent.click(screen.getByRole("button", { name: "Model limits" }))
  fireEvent.change(screen.getByLabelText("Total input tokens"), { target: { value: "8192" } })
  fireEvent.click(screen.getByRole("button", { name: "Save and use" }))
  await screen.findByText("Connection saved and active.")
  expect(saved.profiles.custom).toMatchObject({
    endpoint: { kind: "exact_url", value: "https://example.invalid/exact/path?route=decision" },
    model: "synthetic-custom",
    responseMode: "cloudflare_envelope",
    credential: null,
  })
})

it.each([
  ["jev", "direct"],
  ["ollama", "direct"],
  ["cloudflare", "cloudflare_envelope"],
  ["custom", "direct"],
  ["custom", "cloudflare_envelope"],
] as const)(
  "removes only the saved %s %s credential, not the profile or other drafts",
  async (provider, responseMode) => {
    saved.profiles.target = {
      ...configured(provider),
      responseMode,
      credential: { kind: "connection", id: "target-key" },
    }
    render(<CheckProviderSettings />)
    await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
    fireEvent.change(screen.getByLabelText("TypeSafe API key"), {
      target: { value: "keep-draft" },
    })
    fireEvent.change(screen.getByLabelText("Saved connection"), { target: { value: "target" } })
    fireEvent.click(screen.getByRole("button", { name: "Remove saved credential" }))
    await screen.findByText("Saved credential removed. The connection remains saved.")
    expect(saved.profiles.target).toBeDefined()
    expect(saved.profiles.target!.credential).toBeNull()
    expect(saved.activeId).toBe("jev")
    expect(confirmRemoval).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText("Saved connection"), { target: { value: "jev" } })
    expect(screen.getByLabelText("TypeSafe API key")).toHaveValue("keep-draft")
  },
)

it.each(["ollama", "custom"] as const)(
  "confirms shared legacy-key removal from an inactive %s profile",
  async (provider) => {
    saved = {
      activeId: "jev",
      profiles: {
        jev: { ...configured("jev"), credential: { kind: "legacy_type_safe" } },
        inactive: { ...configured(provider), credential: { kind: "legacy_type_safe" } },
      },
    }
    render(<CheckProviderSettings legacyKeySaved />)
    await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
    fireEvent.change(screen.getByLabelText("Saved connection"), {
      target: { value: "inactive" },
    })
    expect(screen.getByLabelText("Provider")).toHaveValue(provider)
    expect(
      screen.queryByRole("button", { name: "Remove saved credential" }),
    ).not.toBeInTheDocument()

    confirmRemoval.mockResolvedValueOnce(false)
    fireEvent.click(screen.getByRole("button", { name: "Remove shared TypeSafe key" }))
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Remove shared TypeSafe key" })).toBeEnabled(),
    )
    expect(confirmRemoval).toHaveBeenCalledWith(
      "Remove this key? Other connections that use it will stop working.",
      {
        title: "Remove shared TypeSafe key",
        kind: "warning",
        okLabel: "Remove key",
        cancelLabel: "Cancel",
      },
    )
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(["get_system_one_settings"])
    expect(saved.profiles.jev!.credential).toEqual({ kind: "legacy_type_safe" })

    fireEvent.click(screen.getByRole("button", { name: "Remove shared TypeSafe key" }))
    await screen.findByText("Shared TypeSafe key removed.")
    expect(invoke).toHaveBeenCalledWith("remove_typesafe_api_key")
    expect(invoke).not.toHaveBeenCalledWith("remove_system_one_credential", expect.anything())
    expect(saved.activeId).toBe("jev")
    expect(saved.profiles.jev!.credential).toBeNull()
    expect(saved.profiles.inactive!.credential).toBeNull()
    expect(saved.profiles.inactive!.provider).toBe(provider)
    expect(
      screen.queryByRole("button", { name: "Remove shared TypeSafe key" }),
    ).not.toBeInTheDocument()
  },
)

it("refreshes draft limits without saving or switching and keeps unknown limits explicit", async () => {
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
  fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "ollama" } })
  fireEvent.change(screen.getByLabelText("Ollama model"), { target: { value: "draft-model" } })
  fireEvent.click(screen.getByRole("button", { name: "Model limits" }))
  fireEvent.change(screen.getByLabelText("Loaded context tokens"), {
    target: { value: "8192" },
  })
  fireEvent.click(screen.getByRole("button", { name: "Refresh model limits" }))
  await screen.findByText("Model limits refreshed for this draft.")
  expect(screen.getByText("Unknown (unknown)")).toBeVisible()
  expect(screen.getByLabelText("Ollama model")).toHaveValue("draft-model")
  expect(screen.getByLabelText("Loaded context tokens")).toHaveValue(8192)
  expect(saved.activeId).toBe("jev")
  expect(invoke.mock.calls.map(([command]) => command)).toEqual([
    "get_system_one_settings",
    "refresh_system_one_limits",
  ])
})

it("blocks edits and duplicate actions while secure storage saves, then keeps a safe error and the draft", async () => {
  let reject!: (reason: Error) => void
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
  fireEvent.change(screen.getByLabelText("TypeSafe API key"), {
    target: { value: "synthetic-secret" },
  })
  invoke.mockImplementationOnce(
    () =>
      new Promise((_, fail) => {
        reject = fail
      }),
  )
  fireEvent.click(screen.getByRole("button", { name: "Save and use" }))
  expect(screen.getByLabelText("Provider")).toBeDisabled()
  expect(screen.getByLabelText("TypeSafe API key")).toBeDisabled()
  expect(screen.getByRole("button", { name: "Test connection" })).toBeDisabled()
  await act(async () =>
    reject(
      new Error("Secure storage rejected the credential write. Check app access and retry."),
    ),
  )
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Secure storage rejected the credential write",
  )
  expect(screen.getByLabelText("TypeSafe API key")).toHaveValue("synthetic-secret")
  expect(saved.activeId).toBe("jev")
})

it("recovers a failed switch by saving the same unchanged profile", async () => {
  const connection = configured("ollama")
  saved.profiles.local = connection
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
  fireEvent.change(screen.getByLabelText("Saved connection"), { target: { value: "local" } })
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["get_system_one_settings"])

  invoke.mockRejectedValueOnce(new Error("Could not complete the connection update."))
  fireEvent.click(screen.getByRole("button", { name: "Use connection" }))
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not complete the connection update.",
  )
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Save and use" })).toBeEnabled(),
  )
  expect(saved.activeId).toBe("jev")
  expect(screen.getByLabelText("Saved connection")).toHaveValue("local")

  invoke.mockRejectedValueOnce(
    new Error(
      "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks.",
    ),
  )
  fireEvent.click(screen.getByRole("button", { name: "Use connection" }))
  expect(await screen.findByRole("alert")).toHaveTextContent("connection update is incomplete")
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Save and use" })).toBeEnabled(),
  )
  fireEvent.click(screen.getByRole("button", { name: "Save and use" }))
  await screen.findByText("Connection saved and active.")
  expect(saved.activeId).toBe("local")
  expect(invoke).toHaveBeenCalledWith("save_system_one_connection", {
    draft: { connection_id: "local", connection, credential: null },
  })
  expect(invoke.mock.calls.map(([command]) => command)).toEqual([
    "get_system_one_settings",
    "switch_system_one_connection",
    "get_system_one_settings",
    "switch_system_one_connection",
    "get_system_one_settings",
    "save_system_one_connection",
  ])
})

it.each(["ollama", "custom"] as const)(
  "keeps unchanged Save reachable after an unfinalized keyless %s save and reopen",
  async (provider) => {
    saved = { activeId: "target", profiles: { target: configured(provider) } }
    const { unmount } = render(<CheckProviderSettings />)
    await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
    const field = provider === "ollama" ? "Ollama model" : "Custom model"
    fireEvent.change(screen.getByLabelText(field), { target: { value: "persisted-model" } })
    invoke.mockImplementationOnce(async (_command, args) => {
      saved = {
        activeId: args.draft.connection_id,
        profiles: { ...saved.profiles, [args.draft.connection_id]: args.draft.connection },
      }
      throw new Error("Could not complete the connection update.")
    })
    fireEvent.click(screen.getByRole("button", { name: "Save and use" }))
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not complete the connection update.",
    )
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save and use" })).toBeEnabled(),
    )
    expect(saved.profiles.target!.model).toBe("persisted-model")
    expect(saved.profiles.target!.credential).toBeNull()
    expect(screen.getByText("This connection is active.")).toBeVisible()

    unmount()
    invoke.mockClear()
    render(<CheckProviderSettings />)
    await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
    expect(screen.getByLabelText(field)).toHaveValue("persisted-model")
    expect(screen.getByText("This connection is active.")).toBeVisible()
    expect(screen.queryByRole("button", { name: "Use connection" })).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Save and use" })).toBeEnabled()
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(["get_system_one_settings"])

    fireEvent.click(screen.getByRole("button", { name: "Model limits" }))
    invoke.mockRejectedValueOnce(
      new Error(
        "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks.",
      ),
    )
    fireEvent.click(screen.getByRole("button", { name: "Refresh model limits" }))
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "connection update is incomplete",
    )
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save and use" })).toBeEnabled(),
    )
    fireEvent.click(screen.getByRole("button", { name: "Save and use" }))
    await screen.findByText("Connection saved and active.")
    expect(invoke).toHaveBeenCalledWith("save_system_one_connection", {
      draft: { connection_id: "target", connection: saved.profiles.target, credential: null },
    })
    expect(saved.activeId).toBe("target")
    expect(saved.profiles.target!.model).toBe("persisted-model")
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      "get_system_one_settings",
      "refresh_system_one_limits",
      "save_system_one_connection",
    ])
  },
)

it("does not display private provider errors", async () => {
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
  fireEvent.change(screen.getByLabelText("TypeSafe API key"), {
    target: { value: "synthetic-key" },
  })
  invoke.mockRejectedValueOnce(new Error("private endpoint response synthetic-key"))
  fireEvent.click(screen.getByRole("button", { name: "Test connection" }))
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not test the connection")
  expect(screen.queryByText(/private endpoint/)).not.toBeInTheDocument()
})

it("waits for saved connections and offers a read-only retry after a load error", async () => {
  invoke.mockRejectedValueOnce(new Error("offline"))
  render(<CheckProviderSettings />)
  expect(screen.getByLabelText("TypeSafe API key")).toBeDisabled()
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load saved connections")
  fireEvent.click(screen.getByRole("button", { name: "Retry loading connections" }))
  await waitFor(() => expect(screen.getByLabelText("TypeSafe API key")).toBeEnabled())
  expect(invoke.mock.calls.map(([command]) => command)).toEqual([
    "get_system_one_settings",
    "get_system_one_settings",
  ])
})

it("preserves the native pending-credential instruction after a load failure", async () => {
  invoke.mockRejectedValueOnce(
    new Error(
      "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks.",
    ),
  )
  render(<CheckProviderSettings />)
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "connection update is incomplete. Retry it in Settings → Checks.",
  )
  expect(screen.getByLabelText("Provider")).toBeDisabled()
})

it("keeps failed discovery separate from manual limits", async () => {
  saved.profiles.local = configured("ollama")
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
  fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "ollama" } })
  fireEvent.click(screen.getByRole("button", { name: "Model limits" }))
  fireEvent.change(screen.getByLabelText("Total input tokens"), { target: { value: "8192" } })
  invoke.mockRejectedValueOnce(
    new Error("The selected provider does not support this model or runner."),
  )
  fireEvent.click(screen.getByRole("button", { name: "Refresh model limits" }))
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "does not support this model or runner",
  )
  expect(screen.queryByText("Supported")).not.toBeInTheDocument()
  expect(screen.getByLabelText("Total input tokens")).toHaveValue(8192)
  expect(saved.activeId).toBe("jev")
})

it("shows refreshed output support without deriving it from input limits", async () => {
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("Provider")).toBeEnabled())
  fireEvent.click(screen.getByRole("button", { name: "Model limits" }))
  invoke.mockResolvedValueOnce({
    ...capabilities,
    output: { choice: null, score: null, noul: false },
  })
  fireEvent.click(screen.getByRole("button", { name: "Refresh model limits" }))
  await screen.findByText("Model limits refreshed for this draft.")
  expect(screen.getAllByText("Unavailable")).toHaveLength(2)
  expect(screen.queryByText("Supported")).not.toBeInTheDocument()
})

it("searches restored provider controls and excludes past-session controls", () => {
  for (const query of [
    "Ollama",
    "Cloudflare",
    "Custom endpoint",
    "saved connections",
    "manual limits",
  ])
    expect(searchApp(query, "macos").some((entry) => entry.target.kind === "setting")).toBe(
      true,
    )
  expect(
    searchApp("past sessions", "macos").some((entry) => entry.target.kind === "setting"),
  ).toBe(false)
})
