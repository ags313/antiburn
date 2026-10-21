import { fireEvent, render, screen } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import { DEFAULT_SETTINGS } from "../../../../lib/ipc"
import { searchApp } from "../../../../lib/appSearch"
import { CursorAccountSection } from "./SessionSourcesSections"

it.each([
  ["signedIn", "Signed in"],
  ["signedOut", "Signed out"],
  ["expired", "Expired — sign in again in Cursor"],
  ["unreadable", "Login state unavailable"],
] as const)("shows Cursor login state %s without enabling requests", (loginState, label) => {
  const update = vi.fn()
  render(
    <CursorAccountSection
      settings={DEFAULT_SETTINGS}
      update={update}
      loginState={loginState}
    />,
  )
  expect(screen.getByText(new RegExp(label))).toBeInTheDocument()
  const toggle = screen.getByRole("switch", { name: "Read Cursor account usage" })
  expect(toggle).not.toBeChecked()
  expect(screen.getByText(/tokens, cost, model, and time/)).toHaveTextContent(
    "api2.cursor.sh only",
  )
  expect(screen.getByText(/Cursor IDE must be signed in/)).toBeInTheDocument()
  expect(update).not.toHaveBeenCalled()
  fireEvent.click(toggle)
  expect(update).toHaveBeenCalledWith({ cursorAccountUsageEnabled: true })
})

it.each(["macos", "windows", "linux"] as const)(
  "finds and focuses the rendered Cursor setting on %s without toggling it",
  (platform) => {
    const update = vi.fn()
    const { container } = render(
      <CursorAccountSection
        settings={DEFAULT_SETTINGS}
        update={update}
        loginState="signedOut"
      />,
    )
    const result = searchApp("Cursor account", platform).find(
      (item) => item.target.kind === "setting" && item.target.control === "cursorAccountUsage",
    )
    expect(result?.target).toMatchObject({ kind: "setting", control: "cursorAccountUsage" })
    const row = container.querySelector<HTMLElement>(
      '[data-settings-control="cursorAccountUsage"]',
    )!
    row.focus()
    expect(row).toHaveFocus()
    expect(update).not.toHaveBeenCalled()
  },
)
