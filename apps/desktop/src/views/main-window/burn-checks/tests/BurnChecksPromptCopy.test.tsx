import { act, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type * as ClipboardModule from "../../../../lib/clipboard"
import type * as InsightsIpcModule from "../../../../lib/insightsIpc"
import type * as IpcModule from "../../../../lib/ipc"
import { BurnChecksView } from "../../BurnChecksView"
import { CheckPromptAction } from "../BurnCheckDetail"
import { BurnCheckTargetActions } from "../BurnCheckTargetActions"
import { BurnCheckTargetDetail } from "../BurnCheckTargetDetail"
import {
  aggregate,
  deferred,
  installBurnChecksCommandMocks,
  recurredTarget,
  report,
  restoreBurnChecksTestWindow,
  setup,
  target,
} from "./burnChecksTestSupport"

const commands = vi.hoisted(() => ({
  prepare: vi.fn(),
  apply: vi.fn(),
  copy: vi.fn(),
  copyFallback: vi.fn(),
  copyBatch: vi.fn(),
  writeClipboardText: vi.fn(),
  openSample: vi.fn(),
  noteInteraction: vi.fn(),
}))

vi.mock("../../../../lib/insightsIpc", async (importOriginal) => ({
  ...(await importOriginal<typeof InsightsIpcModule>()),
  prepareAutoFixBurnCheckTarget: commands.prepare,
  applyPreparedBurnCheckOperation: commands.apply,
  copyPromptFixBurnCheckTarget: commands.copy,
  copyPromptFixBurnCheck: commands.copyFallback,
  copyPromptFixBurnCheckTargets: commands.copyBatch,
  openBurnCheckSample: commands.openSample,
}))

vi.mock("../../../../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof IpcModule>()),
  noteInteraction: commands.noteInteraction,
}))

vi.mock("../../../../lib/clipboard", async (importOriginal) => ({
  ...(await importOriginal<typeof ClipboardModule>()),
  writeClipboardText: commands.writeClipboardText,
}))

beforeEach(() => installBurnChecksCommandMocks(commands))
afterEach(restoreBurnChecksTestWindow)

describe("Burn Check prompt copy", () => {
  it("keeps one pending target copy through handle, watch creation, and recurrence updates", async () => {
    const pending = deferred<InsightsIpcModule.CopyPromptFixBurnCheckTargetOutcome | null>()
    commands.copy.mockReturnValueOnce(pending.promise)
    const view = render(<BurnCheckTargetActions target={target} refresh={vi.fn()} />)
    const button = screen.getByRole("button", { name: "Copy fix prompt" })
    fireEvent.click(button)
    fireEvent.click(button)

    for (const updated of [
      { ...target, actionId: "rotated-handle" },
      {
        ...recurredTarget("watch-created"),
        watch: {
          watchId: "watch-1",
          origin: "action" as const,
          lifecycle: "waitingForPromptUse" as const,
          verification: { status: "watching" as const },
          savings: { status: "pending" as const },
        },
      },
      recurredTarget("recurred-handle"),
    ]) {
      view.rerender(<BurnCheckTargetActions target={updated} refresh={vi.fn()} />)
      expect(screen.getByRole("button", { name: "Copying…" })).toBeDisabled()
      fireEvent.click(button)
    }

    await act(async () =>
      pending.resolve({ outcome: "promptReady", prompt: "Current prompt", watch: null }),
    )
    expect(commands.copy).toHaveBeenCalledExactlyOnceWith("oldModelUsage", "finding-stable")
    expect(commands.writeClipboardText).toHaveBeenCalledExactlyOnceWith("Current prompt")
    expect(screen.getByRole("button", { name: "Copied" })).toBeEnabled()
  })

  it("prepares a fresh target prompt after a native write failure", async () => {
    commands.copy
      .mockResolvedValueOnce({ outcome: "promptReady", prompt: "First prompt", watch: null })
      .mockResolvedValueOnce({ outcome: "promptReady", prompt: "Latest prompt", watch: null })
    commands.writeClipboardText.mockRejectedValueOnce(new Error("Denied"))
    render(<BurnCheckTargetActions target={target} refresh={vi.fn()} />)

    fireEvent.click(screen.getByRole("button", { name: "Copy fix prompt" }))
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The fix prompt could not be copied. Try again.",
    )
    fireEvent.click(screen.getByRole("button", { name: "Copy fix prompt" }))
    await screen.findByRole("button", { name: "Copied" })

    expect(commands.copy).toHaveBeenCalledTimes(2)
    expect(commands.writeClipboardText).toHaveBeenNthCalledWith(1, "First prompt")
    expect(commands.writeClipboardText).toHaveBeenNthCalledWith(2, "Latest prompt")
    expect(commands.noteInteraction.mock.calls).toEqual([
      [{ kind: "burnCheckPromptPrepared", outcome: "ready" }],
      [{ kind: "burnCheckPromptPrepared", outcome: "ready" }],
      [{ kind: "burnCheckPromptCopied" }],
    ])
  })

  it("keeps a pending report copy through waiting, fixed, and recurred lifecycle updates", async () => {
    const pending = deferred<InsightsIpcModule.CopyPromptFixBurnCheckOutcome | null>()
    commands.copyFallback.mockReturnValueOnce(pending.promise)
    const check = report.categories[0]!
    const { adapter, session } = setup(target, false, aggregate, {
      ...report,
      categories: [check],
    })
    fireEvent.click(await screen.findByRole("button", { name: "Copy fix prompt" }))

    for (const lifecycle of ["awaitingVerification", "passing", "failing"] as const) {
      vi.mocked(adapter.getReport).mockResolvedValue({
        ...report,
        categories: [{ ...check, lifecycle, finding: lifecycle === "failing" ? 1 : 0 }],
      })
      await act(async () => session.refresh())
      const copying = screen.getByRole("button", { name: "Preparing…" })
      expect(copying).toBeDisabled()
      fireEvent.click(copying)
      if (lifecycle !== "failing") {
        expect(screen.queryByRole("button", { name: "Fix" })).not.toBeInTheDocument()
      }
    }

    await act(async () =>
      pending.resolve({ outcome: "promptReady", prompt: "Fresh report prompt" }),
    )
    expect(commands.copyFallback).toHaveBeenCalledExactlyOnceWith("oldModelUsage")
    expect(commands.copyBatch).not.toHaveBeenCalled()
    expect(commands.writeClipboardText).toHaveBeenCalledExactlyOnceWith("Fresh report prompt")
    expect(screen.getByRole("button", { name: "Copied" })).toBeEnabled()
  })

  it.each(["finding", "saved win"] as const)(
    "loads a passing check with a known %s on startup",
    async (known) => {
      const { adapter } = setup(
        target,
        false,
        known === "saved win" ? aggregate : { wins: [] },
        {
          ...report,
          categories: [
            {
              ...report.categories[0]!,
              lifecycle: "passing",
              finding: known === "finding" ? 1 : 0,
            },
          ],
        },
      )
      const copy = await screen.findByRole("button", { name: "Copy fix prompt" })
      expect(adapter.getTargets).toHaveBeenCalledWith("oldModelUsage")
      expect(copy).toBeEnabled()
      expect(screen.queryByRole("button", { name: "Fix" })).not.toBeInTheDocument()
      fireEvent.click(copy)
      await screen.findByRole("button", { name: "Copied" })
      expect(commands.copyFallback).toHaveBeenCalledExactlyOnceWith("oldModelUsage")
    },
  )

  it("does not load or offer a prompt for an unrelated empty passing check", async () => {
    const { adapter } = setup(null, false, aggregate, {
      ...report,
      categories: [report.categories[1]!],
    })
    await screen.findByRole("button", { name: "Unused skills, Passed" })
    expect(adapter.getTargets).not.toHaveBeenCalled()
    expect(screen.queryByRole("button", { name: "Copy fix prompt" })).not.toBeInTheDocument()
  })

  it("cancels a pending copy when its mounted detail is hidden and does not clear a newer request", async () => {
    const old = deferred<InsightsIpcModule.CopyPromptFixBurnCheckOutcome | null>()
    const current = deferred<InsightsIpcModule.CopyPromptFixBurnCheckOutcome | null>()
    commands.copyFallback.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise)
    setup(target, false, aggregate, report)
    fireEvent.click(await screen.findByRole("button", { name: "Copy fix prompt" }))
    const detail = document.getElementById("burn-check-oldModelUsage-detail")!

    fireEvent.click(screen.getByRole("button", { name: "Unused skills, Passed" }))
    expect(detail).toHaveAttribute("hidden")
    fireEvent.click(screen.getByRole("button", { name: /Old model usage, 1 failed/ }))
    expect(document.getElementById("burn-check-oldModelUsage-detail")).toBe(detail)
    fireEvent.click(screen.getByRole("button", { name: "Copy fix prompt" }))

    await act(async () => old.resolve({ outcome: "promptReady", prompt: "Obsolete prompt" }))
    expect(commands.writeClipboardText).not.toHaveBeenCalled()
    expect(screen.getByRole("button", { name: "Preparing…" })).toBeDisabled()

    await act(async () => current.resolve({ outcome: "promptReady", prompt: "Current prompt" }))
    expect(commands.writeClipboardText).toHaveBeenCalledExactlyOnceWith("Current prompt")
    expect(screen.getByRole("button", { name: "Copied" })).toBeEnabled()
  })

  it("cancels preparation when the Checks section becomes inactive", async () => {
    const pending = deferred<InsightsIpcModule.CopyPromptFixBurnCheckOutcome | null>()
    commands.copyFallback.mockReturnValueOnce(pending.promise)
    const { session, view } = setup(target, false, aggregate, report)
    fireEvent.click(await screen.findByRole("button", { name: "Copy fix prompt" }))
    view.rerender(<BurnChecksView active={false} session={session} />)
    view.rerender(<BurnChecksView active session={session} />)

    await act(async () =>
      pending.resolve({ outcome: "promptReady", prompt: "Obsolete prompt" }),
    )
    expect(commands.writeClipboardText).not.toHaveBeenCalled()
    expect(screen.getByRole("button", { name: "Copy fix prompt" })).toBeEnabled()
  })

  it("does not clear a newer copy when an older native write completes", async () => {
    const oldWrite = deferred<void>()
    const currentPrompt = deferred<InsightsIpcModule.CopyPromptFixBurnCheckOutcome | null>()
    commands.writeClipboardText.mockReturnValueOnce(oldWrite.promise)
    const view = render(
      <CheckPromptAction detector="oldModelUsage" targets={[target]} refresh={vi.fn()} />,
    )
    fireEvent.click(screen.getByRole("button", { name: "Copy fix prompt" }))
    await act(async () => undefined)
    expect(commands.writeClipboardText).toHaveBeenCalledOnce()
    view.rerender(
      <CheckPromptAction
        detector="oldModelUsage"
        targets={[target]}
        refresh={vi.fn()}
        active={false}
      />,
    )
    view.rerender(
      <CheckPromptAction detector="oldModelUsage" targets={[target]} refresh={vi.fn()} />,
    )
    commands.copyFallback.mockReturnValueOnce(currentPrompt.promise)
    fireEvent.click(screen.getByRole("button", { name: "Copy fix prompt" }))

    await act(async () => oldWrite.resolve())
    expect(screen.getByRole("button", { name: "Preparing…" })).toBeDisabled()
    await act(async () =>
      currentPrompt.resolve({ outcome: "promptReady", prompt: "Latest prompt" }),
    )
    expect(screen.getByRole("button", { name: "Copied" })).toBeEnabled()
    expect(
      commands.noteInteraction.mock.calls.filter(
        ([event]) => event.kind === "burnCheckPromptCopied",
      ),
    ).toHaveLength(2)
  })

  it.each([
    ["waitingForPromptUse", "watching"],
    ["watching", "verificationUnavailable"],
  ] as const)("does not show prompt watch status for %s", (lifecycle, status) => {
    const view = render(
      <BurnCheckTargetDetail
        target={{
          ...target,
          watch: {
            watchId: "watch-prompt",
            origin: "action",
            lifecycle,
            verification: { status },
            savings: { status: "pending" },
          },
        }}
        refresh={vi.fn()}
      />,
    )
    expect(screen.getByRole("button", { name: "Copy fix prompt" })).toBeEnabled()
    expect(view.container).not.toHaveTextContent(
      /prompt is ready|could not confirm this change|list was refreshed/i,
    )
  })
})
