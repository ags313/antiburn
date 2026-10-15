import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { createElement } from "react"
import * as snoozes from "../../../../lib/snoozedBurnChecks"
import * as ipc from "../../../../lib/insightsIpc"
import { BurnChecksView } from "../../BurnChecksView"
import { installedCheckFixture as installed } from "./installedCheckFixture"
import { aggregate, report, setup, target } from "./burnChecksTestSupport"

beforeEach(() => {
  HTMLElement.prototype.scrollIntoView = vi.fn()
  vi.spyOn(snoozes, "useSnoozedBurnChecks").mockReturnValue({ status: "ready", records: [] })
})
afterEach(() => vi.restoreAllMocks())

describe("installed checks in the existing UI", () => {
  it("shows active assessment progress before the first saved result", async () => {
    const { session, adapter } = setup(
      null,
      false,
      { wins: [] },
      {
        ...report,
        categories: [],
        installedChecks: [
          { ...installed, finding: 0, clean: 0, checkingCount: 1, targets: [] },
        ],
      },
    )
    fireEvent.click(await screen.findByRole("button", { name: /Not assessed/ }))
    const checkingRow = await screen.findByRole("button", { name: /Failed Retry Loop/ })
    fireEvent.click(checkingRow)
    expect(screen.getByRole("heading", { name: "Failed Retry Loop" })).toBeVisible()
    expect(screen.getAllByText("1 checking")).toHaveLength(2)
    expect(screen.getByText("This check has not checked any sessions yet.")).toBeVisible()
    expect(screen.queryByText(/Scanning sessions/)).not.toBeInTheDocument()
    vi.mocked(adapter.getReport).mockResolvedValue({
      ...report,
      categories: [],
      installedChecks: [
        {
          ...installed,
          finding: 0,
          clean: 1,
          checkingCount: 0,
          targets: [],
          explanation: "No repeated failure found.",
        },
      ],
    })
    await act(async () => session.refresh())
    expect(
      await screen.findByRole("button", { name: "Failed Retry Loop, Passed" }),
    ).toBeVisible()
    expect(screen.getByText("No repeated failure found.")).toBeVisible()
    expect(screen.queryByText("1 checking")).not.toBeInTheDocument()
  })

  it("shows when a check failed or completed without changing status", async () => {
    const { adapter, session } = setup(
      null,
      false,
      { wins: [] },
      {
        ...report,
        categories: [],
        installedChecks: [
          {
            ...installed,
            finding: 0,
            clean: 0,
            reviewFailed: 1,
            checkingCount: 0,
            targets: [],
          },
        ],
      },
    )
    fireEvent.click(await screen.findByRole("button", { name: /Not assessed/ }))
    const row = await screen.findByRole("button", { name: /Failed Retry Loop/ })
    fireEvent.click(row)
    expect(
      screen.getByText(
        "The check could not complete for 1 session. New tool results may trigger another check.",
      ),
    ).toBeVisible()

    vi.mocked(adapter.getReport).mockResolvedValue({
      ...report,
      categories: [],
      installedChecks: [
        {
          ...installed,
          finding: 0,
          clean: 0,
          reviewFailed: 0,
          reviewedWithoutStatus: 1,
          checkingCount: 0,
          targets: [],
        },
      ],
    })
    await act(async () => session.refresh())
    expect(
      await screen.findByText("The check completed for 1 session without changing status."),
    ).toBeVisible()
  })

  it("uses ordinary counts, keyboard selection and the right detail without native actions", async () => {
    const { adapter } = setup(null, false, aggregate, {
      ...report,
      installedChecks: [installed],
    })
    const native = await screen.findByRole("button", { name: /Old model usage, / })
    expect(
      screen.getByLabelText("4 check types in total, including snoozed checks"),
    ).toBeVisible()
    expect(screen.getByRole("region", { name: "Failed checks 2" })).toBeVisible()
    fireEvent.keyDown(native, { key: "ArrowDown" })
    const row = screen.getByRole("button", { name: "Failed Retry Loop, 1 failed · 1 passed" })
    expect(row).toHaveFocus()
    expect(row).toHaveAttribute("aria-pressed", "true")
    fireEvent.keyDown(row, { key: "Enter" })
    await waitFor(() =>
      expect(
        document.getElementById(`burn-check-${installed.effectiveId}-detail`),
      ).toHaveFocus(),
    )
    const detail = within(screen.getByRole("region", { name: "Burn check details" }))
    expect(detail.getByRole("heading", { name: installed.name, level: 2 })).toBeVisible()
    expect(detail.getByRole("button", { name: "Snooze" })).toBeVisible()
    expect(detail.queryByRole("button", { name: "Fix" })).not.toBeInTheDocument()
    expect(detail.queryByRole("button", { name: "Copy fix prompt" })).not.toBeInTheDocument()
    expect(adapter.getTargets).not.toHaveBeenCalledWith(installed.effectiveId)
    const selectedDetail = within(
      document.getElementById(`burn-check-${installed.effectiveId}-detail`)!,
    )
    expect(
      selectedDetail.queryByText(/estimated burn|wasted|Savings|Awaiting verification/),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: /Reload|Pause|Resume/ }),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByText(/Installed checks|Assessment history|Generation:|Probabilities:/),
    ).not.toBeInTheDocument()
  })

  it("uses existing source excerpts and session cards through read-only routes", async () => {
    const evidence = vi.spyOn(ipc, "getInstalledCheckEvidence").mockResolvedValue({
      status: "available",
      items: [
        {
          label: "context",
          sourceLabel: "Requested task",
          reference: "task",
          observedAtMs: null,
          startLine: null,
          endLine: null,
          excerpt: "Run the tests",
          explanation: "",
          limitation: null,
        },
      ],
    })
    const open = vi.spyOn(ipc, "openBurnCheckSample").mockResolvedValue({ outcome: "opened" })
    const samples = target.samples
    setup(
      null,
      false,
      { wins: [] },
      {
        ...report,
        categories: [],
        installedChecks: [{ ...installed, targets: [{ ...installed.targets[0]!, samples }] }],
      },
    )
    fireEvent.click(await screen.findByRole("button", { name: "Show source details" }))
    expect(await screen.findByText("Run the tests")).toBeVisible()
    expect(evidence).toHaveBeenCalledWith(installed.targets[0]!.id)
    expect(screen.getByRole("button", { name: "Hide source details" })).toHaveAttribute(
      "aria-expanded",
      "true",
    )
    const card = within(screen.getByRole("region", { name: "Failed sessions" })).getByRole(
      "button",
    )
    fireEvent.click(card)
    await waitFor(() => expect(open).toHaveBeenCalledWith(samples[0]!.navigationHandle))
  })

  it("preserves selection through a pass transition and treats invalid results as unassessed", async () => {
    const { session, adapter, view } = setup(
      null,
      false,
      { wins: [] },
      { ...report, categories: [], installedChecks: [installed] },
    )
    view.rerender(
      createElement(BurnChecksView, {
        active: true,
        session,
        focusedCheck: installed.effectiveId,
        focusRevision: 1,
      }),
    )
    await screen.findByRole("button", { name: /Failed Retry Loop, 1 failed/ })
    vi.mocked(adapter.getReport).mockResolvedValue({
      ...report,
      categories: [],
      installedChecks: [
        {
          ...installed,
          finding: 0,
          clean: 2,
          targets: [],
          explanation: "No concern in the checked work.",
        },
      ],
    })
    await act(async () => session.refresh())
    expect(
      await screen.findByRole("button", { name: "Failed Retry Loop, Passed" }),
    ).toHaveAttribute("aria-pressed", "true")
    expect(screen.getByText("No concern in the checked work.")).toBeVisible()
    expect(screen.queryByRole("region", { name: "Savings" })).not.toBeInTheDocument()
    vi.mocked(adapter.getReport).mockResolvedValue({
      ...report,
      categories: [],
      installedChecks: [
        {
          ...installed,
          finding: 0,
          clean: 0,
          targets: [],
          availability: { status: "invalid", reason: "Invalid YAML." },
          diagnostic: "Invalid YAML.",
        },
      ],
    })
    await act(async () => session.refresh())
    expect(
      await screen.findByRole("button", { name: "Failed Retry Loop, Not assessed" }),
    ).toBeVisible()
    expect(screen.getByRole("button", { name: "Not assessed (1)" })).toHaveAttribute(
      "aria-expanded",
      "true",
    )
    expect(screen.getByText("Invalid YAML.")).toBeVisible()
  })

  it("uses logical installed snoozes, preserves totals and restores the ordinary group", async () => {
    const snooze = vi.spyOn(snoozes, "snoozeBurnCheck").mockResolvedValue()
    const unsnooze = vi.spyOn(snoozes, "unsnoozeBurnCheck").mockResolvedValue()
    const { session, view } = setup(
      null,
      false,
      { wins: [] },
      { ...report, categories: [], installedChecks: [installed] },
    )
    fireEvent.keyDown(await screen.findByRole("button", { name: "Snooze" }), {
      key: "ArrowDown",
    })
    fireEvent.click(await screen.findByRole("menuitem", { name: "For 1 week" }))
    expect(snooze).toHaveBeenCalledWith(installed.effectiveId, "week")
    vi.mocked(snoozes.useSnoozedBurnChecks).mockReturnValue({
      status: "ready",
      records: [{ detector: installed.effectiveId, scope: "check", until: null }],
    })
    view.rerender(
      createElement(BurnChecksView, {
        active: true,
        session,
        focusedCheck: installed.effectiveId,
        focusRevision: 1,
      }),
    )
    expect(
      screen.getByLabelText("1 check type in total, including snoozed checks"),
    ).toBeVisible()
    expect(screen.queryByRole("heading", { name: /Failed checks/ })).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Snoozed 1" })).toHaveAttribute(
      "aria-expanded",
      "true",
    )
    fireEvent.click(screen.getByRole("button", { name: "Unsnooze" }))
    expect(unsnooze).toHaveBeenCalledWith(installed.effectiveId)
    vi.mocked(snoozes.useSnoozedBurnChecks).mockReturnValue({ status: "ready", records: [] })
    view.rerender(
      createElement(BurnChecksView, {
        active: true,
        session,
        focusedCheck: installed.effectiveId,
        focusRevision: 2,
      }),
    )
    expect(screen.getByRole("region", { name: "Failed checks 1" })).toBeVisible()
  })
})
