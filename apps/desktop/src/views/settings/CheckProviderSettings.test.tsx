import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { beforeEach, expect, it, vi } from "vitest"

import { defaultConnection, type ProviderSettings } from "../../lib/smartCheckProviders"
import { searchApp } from "../../lib/appSearch"
import { CheckProviderSettings } from "./CheckProviderSettings"

const invoke = vi.hoisted(() => vi.fn())
vi.mock("@tauri-apps/api/core", () => ({ invoke }))

let saved: ProviderSettings

beforeEach(() => {
  saved = { activeId: "jev", profiles: { jev: defaultConnection("jev") } }
  invoke.mockReset()
  invoke.mockImplementation(async (command, args) => {
    if (command === "get_system_one_settings") return saved
    if (command === "test_system_one_connection") return null
    if (command === "save_system_one_connection") {
      saved = {
        activeId: args.draft.connection_id,
        profiles: { ...saved.profiles, [args.draft.connection_id]: args.draft.connection },
      }
      return saved
    }
    return null
  })
})

it("shows one Jev connection flow and hides provider, saved-connection, and model-limit controls", async () => {
  saved.profiles.ollama = { ...defaultConnection("ollama"), model: "legacy-model" }
  saved.activeId = "ollama"
  render(<CheckProviderSettings />)
  await waitFor(() => expect(screen.getByLabelText("TypeSafe API key")).toBeEnabled())

  expect(screen.getByRole("heading", { name: "Decision model" })).toBeVisible()
  expect(screen.getByLabelText("Jev model")).toHaveValue(defaultConnection("jev").model)
  expect(screen.queryByLabelText("Smart check provider")).not.toBeInTheDocument()
  expect(screen.queryByLabelText("Saved connections")).not.toBeInTheDocument()
  expect(screen.queryByRole("button", { name: "Refresh model limits" })).not.toBeInTheDocument()
  expect(screen.queryByLabelText("Manual total input tokens")).not.toBeInTheDocument()
  expect(screen.queryByRole("button", { name: "Use saved connection" })).not.toBeInTheDocument()

  fireEvent.change(screen.getByLabelText("TypeSafe API key"), {
    target: { value: "synthetic-secret" },
  })
  fireEvent.click(screen.getByRole("button", { name: "Test connection" }))
  await screen.findByText("Connection test passed.")
  expect(invoke).toHaveBeenCalledWith(
    "test_system_one_connection",
    expect.objectContaining({
      draft: expect.objectContaining({ connection_id: "jev", credential: "synthetic-secret" }),
    }),
  )
  fireEvent.click(screen.getByRole("button", { name: "Save and use connection" }))
  await screen.findByText("Connection saved and selected for Model-based Checks.")
  expect(saved.activeId).toBe("jev")
  expect(saved.profiles.ollama?.model).toBe("legacy-model")
})

it("searches only for visible model-based check controls", () => {
  const result = searchApp("Model-based Checks", "macos").find(
    (entry) => entry.target.kind === "setting" && entry.target.control === "smartChecksEnabled",
  )
  expect(result?.label).toBe("Model-based Checks")
  for (const query of ["past sessions", "Ollama", "saved connections", "manual limits"]) {
    expect(searchApp(query, "macos").some((entry) => entry.target.kind === "setting")).toBe(
      false,
    )
  }
})
