import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { beforeEach, expect, it, vi } from "vitest"
import { defaultConnection, type ProviderSettings } from "../../../lib/smartCheckProviders"
import { CheckProviderSettings } from "../CheckProviderSettings"
import { ConnectionActions } from "../ConnectionActions"
import { ConnectionDialog } from "../ConnectionDialog"
import { providerSettingsFixture } from "./providerSettingsFixture"

const invoke = vi.hoisted(() => vi.fn())
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => true) }))

let saved: ProviderSettings

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((complete, fail) => {
    resolve = complete
    reject = fail
  })
  return { promise, resolve, reject }
}

async function nextFrame() {
  await act(async () => {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
  })
}

async function renderSettings() {
  render(
    <>
      <CheckProviderSettings />
      <button type="button">Check files</button>
    </>,
  )
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "More actions for Jev 2" })).toBeEnabled(),
  )
  return screen.getByRole("button", { name: "Check files" })
}

beforeEach(() => {
  HTMLElement.prototype.scrollIntoView = vi.fn()
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true
  }
  HTMLDialogElement.prototype.close = function () {
    this.open = false
  }
  const profile = {
    ...defaultConnection("jev"),
    credential: { kind: "connection" as const, id: "synthetic-key" },
  }
  saved = providerSettingsFixture({
    activeId: "personal",
    profiles: { personal: profile, work: profile },
  })
  invoke.mockReset().mockImplementation(async (command) => {
    if (command === "get_system_one_settings") return saved
    throw new Error(`Unexpected command: ${command}`)
  })
})

it("keeps a row menu open through the full pointer click before Edit", async () => {
  await renderSettings()
  const trigger = screen.getByRole("button", { name: "More actions for Jev 2" })
  fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false })
  await screen.findByRole("menu")
  fireEvent.pointerUp(trigger, { button: 0, ctrlKey: false })
  fireEvent.click(trigger)
  await nextFrame()
  expect(trigger).toHaveAttribute("aria-expanded", "true")
  expect(screen.getByRole("menu")).toHaveAttribute("data-state", "open")
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit" }))
  const dialog = await screen.findByRole("dialog", { name: "Edit Jev 2 connection" })
  expect(screen.queryByRole("menu")).not.toBeInTheDocument()
  await nextFrame()
  expect(within(dialog).getByRole("button", { name: "Change" })).toHaveFocus()
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }))
  await waitFor(() => expect(trigger).toHaveFocus())
})

it("keeps a new row menu focused when delayed dialog cleanup runs after its opener is enabled", async () => {
  const frames: FrameRequestCallback[] = []
  const requestFrame = vi
    .spyOn(window, "requestAnimationFrame")
    .mockImplementation((callback) => frames.push(callback))
  const onEdit = vi.fn()
  const onRemove = vi.fn()
  function panel(open: boolean, disabled = false) {
    return (
      <>
        <ConnectionActions
          id="connection-actions"
          label="Jev"
          disabled={disabled}
          pending={false}
          onEdit={onEdit}
          onRemove={onRemove}
        />
        {open && (
          <ConnectionDialog title="Edit Jev connection" onCancel={vi.fn()}>
            <button type="button">Cancel</button>
          </ConnectionDialog>
        )}
      </>
    )
  }
  try {
    const { rerender } = render(panel(false))
    const trigger = screen.getByRole("button", { name: "More actions for Jev" })
    trigger.focus()
    rerender(panel(true))
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus()
    rerender(panel(true, true))
    expect(trigger).toBeDisabled()
    rerender(panel(false, true))
    expect(trigger).toBeDisabled()
    expect(frames).toHaveLength(1)
    const restoreFocus = frames[0]!
    rerender(panel(false))
    trigger.focus()
    fireEvent.keyDown(trigger, { key: "Enter" })
    const edit = await screen.findByRole("menuitem", { name: "Edit" })
    act(() => edit.focus())
    expect(trigger).toHaveAttribute("aria-expanded", "true")
    expect(edit).toHaveFocus()
    act(() => restoreFocus(0))
    expect(edit).toHaveFocus()
    expect(trigger).toHaveAttribute("aria-expanded", "true")
    expect(screen.getByRole("menu")).toHaveAttribute("data-state", "open")
    expect(onEdit).not.toHaveBeenCalled()
    expect(onRemove).not.toHaveBeenCalled()
  } finally {
    requestFrame.mockRestore()
  }
})

it.each(["success", "pending failure"])(
  "preserves a later focus choice when Use ends with %s",
  async (outcome) => {
    const files = await renderSettings()
    const operation = deferred<ProviderSettings>()
    invoke.mockImplementationOnce((command) => {
      expect(command).toBe("switch_system_one_connection")
      return operation.promise
    })
    fireEvent.click(screen.getByRole("button", { name: "Use Jev 2" }))
    files.focus()
    await act(async () => {
      if (outcome === "success") {
        saved = { ...saved, activeId: "work" }
        operation.resolve(saved)
      } else {
        saved = {
          ...saved,
          pendingOperation: {
            id: "pending-switch",
            kind: "switch",
            connectionId: "work",
            requestId: null,
          },
        }
        operation.reject(
          new Error(
            "Could not complete the connection operation. Use Retry in Settings → Checks.",
          ),
        )
      }
    })
    if (outcome === "success")
      expect(screen.getByRole("listitem", { name: "Jev 2" })).toHaveAccessibleDescription(
        "Selected",
      )
    else
      expect(screen.getByRole("button", { name: "Retry connection operation" })).toBeEnabled()
    await nextFrame()
    expect(files).toHaveFocus()
  },
)

it.each(["success", "failure", "pending failure"])(
  "preserves a later focus choice when Remove ends with %s",
  async (outcome) => {
    const files = await renderSettings()
    const operation = deferred<ProviderSettings>()
    invoke.mockImplementationOnce((command) => {
      expect(command).toBe("remove_system_one_connection")
      return operation.promise
    })
    fireEvent.keyDown(screen.getByRole("button", { name: "More actions for Jev 2" }), {
      key: "Enter",
    })
    fireEvent.click(await screen.findByRole("menuitem", { name: "Remove" }))
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("remove_system_one_connection", {
        connectionId: "work",
        revision: 1,
      }),
    )
    files.focus()
    await act(async () => {
      if (outcome === "success") {
        saved = { ...saved, profiles: { personal: saved.profiles.personal! } }
        operation.resolve(saved)
      } else {
        if (outcome === "pending failure")
          saved = {
            ...saved,
            pendingOperation: {
              id: "pending-remove",
              kind: "remove",
              connectionId: "work",
              requestId: null,
            },
          }
        operation.reject(
          new Error("Could not remove the connection. Reload connections and try again."),
        )
      }
    })
    if (outcome === "success")
      expect(screen.queryByRole("listitem", { name: "Jev 2" })).not.toBeInTheDocument()
    else if (outcome === "pending failure")
      expect(screen.getByRole("button", { name: "Retry connection operation" })).toBeEnabled()
    else expect(screen.getByRole("alert")).toBeVisible()
    await nextFrame()
    expect(files).toHaveFocus()
  },
)

it.each(["completed", "failure"])(
  "preserves a later focus choice when Retry ends with %s",
  async (outcome) => {
    saved = {
      ...saved,
      pendingOperation: {
        id: "pending-switch",
        kind: "switch",
        connectionId: "work",
        requestId: null,
      },
    }
    const files = await renderSettings()
    const operation = deferred<{ settings: ProviderSettings; outcome: "completed" }>()
    invoke.mockImplementationOnce((command) => {
      expect(command).toBe("retry_system_one_connection_operation")
      return operation.promise
    })
    fireEvent.click(screen.getByRole("button", { name: "Retry connection operation" }))
    files.focus()
    await act(async () => {
      if (outcome === "completed") {
        saved = { ...saved, pendingOperation: null }
        operation.resolve({ settings: saved, outcome: "completed" })
      } else
        operation.reject(
          new Error(
            "Could not complete the connection operation. Use Retry in Settings → Checks.",
          ),
        )
    })
    if (outcome === "completed")
      expect(
        screen.queryByRole("button", { name: "Retry connection operation" }),
      ).not.toBeInTheDocument()
    else
      expect(screen.getByRole("button", { name: "Retry connection operation" })).toBeEnabled()
    await nextFrame()
    expect(files).toHaveFocus()
  },
)
