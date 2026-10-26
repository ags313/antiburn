import { fireEvent, screen, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type {
  InstalledCheckEvent,
  InstalledCheckReportPayload,
} from "../../../../lib/insightsIpc"
import * as snoozes from "../../../../lib/snoozedBurnChecks"
import { report, setup } from "./burnChecksTestSupport"
import { installedCheckFixture as installed } from "./installedCheckFixture"

const eventCases: Array<{
  event: InstalledCheckEvent
  singular: string
  plural: string
  limit?: string
}> = [
  { event: "toolResult", singular: "tool result", plural: "tool results" },
  { event: "userMessage", singular: "user message", plural: "user messages" },
  { event: "assistantMessage", singular: "assistant message", plural: "assistant messages" },
  { event: "toolCall", singular: "tool call", plural: "tool calls" },
  { event: "message", singular: "message", plural: "messages" },
  { event: "turnEnd", singular: "response cycle", plural: "response cycles" },
  {
    event: "sessionStart",
    singular: "session-start observation",
    plural: "session-start observations",
    limit:
      "Session-start observations mark when antiburn attaches to a session, not when the agent starts or resumes.",
  },
  {
    event: "sessionIdle",
    singular: "idle observation",
    plural: "idle observations",
    limit:
      "Idle observations record at least 120 seconds without observed activity. They do not prove that the agent finished.",
  },
  {
    event: "sessionEvent",
    singular: "session observation",
    plural: "session observations",
    limit:
      "Session observations record attachment or observed inactivity. They do not prove that the agent started, resumed, or finished.",
  },
]

beforeEach(() => {
  HTMLElement.prototype.scrollIntoView = vi.fn()
  vi.spyOn(snoozes, "useSnoozedBurnChecks").mockReturnValue({ status: "ready", records: [] })
})

afterEach(() => vi.restoreAllMocks())

function setupWait(
  event: InstalledCheckEvent,
  count: number,
  catchingUp = false,
  overrides: Partial<InstalledCheckReportPayload> = {},
) {
  return setup(
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
          hasStarted: false,
          reviewedWithoutStatus: 0,
          waitingForResultsCount: 0,
          waitingForEvents: { event, count, catchingUp },
          targets: [],
          ...overrides,
        },
      ],
    },
  )
}

describe("installed check event waits", () => {
  it.each(
    [
      { result: "Passed", finding: 0, clean: 1, reviewedWithoutStatus: 0 },
      { result: "1 failed", finding: 1, clean: 0, reviewedWithoutStatus: 0 },
      { result: "Not assessed", finding: 0, clean: 0, reviewedWithoutStatus: 1 },
    ].flatMap((saved) =>
      ["waiting", "catching up", "checking"].map((progress) => ({ ...saved, progress })),
    ),
  )(
    "keeps $result and its $progress progress visible in detail",
    async ({ result, finding, clean, reviewedWithoutStatus, progress }) => {
      setupWait("sessionIdle", 3, progress === "catching up", {
        finding,
        clean,
        reviewedWithoutStatus,
        hasStarted: true,
        checkingCount: progress === "checking" ? 1 : 0,
        explanation: "Saved assessment result.",
        targets: finding > 0 ? installed.targets : [],
      })
      const collection = within(
        await screen.findByRole("region", { name: "Burn check collection" }),
      )
      if (result !== "1 failed") {
        const group = collection.getByRole("button", {
          name: result === "Passed" ? "Passed checks 1" : "Not assessed (1)",
        })
        if (group.getAttribute("aria-expanded") !== "true") fireEvent.click(group)
      }
      const status = `${result}${progress === "checking" ? " · Checking" : ""}`
      fireEvent.click(collection.getByRole("button", { name: `Failed retry loop, ${status}` }))
      const detail = document.getElementById(`burn-check-${installed.effectiveId}-detail`)!
      expect(detail.querySelector("header")).toHaveTextContent(status)
      if (progress === "checking") {
        expect(within(detail).getByText("This check is queued or running.")).toBeVisible()
      } else if (progress === "catching up") {
        expect(
          within(detail).getByText("Session activity is still being processed."),
        ).toBeVisible()
        expect(detail).not.toHaveTextContent(/needs 3 more/)
      } else {
        expect(detail).toHaveTextContent("It needs 3 more idle observations")
        expect(
          within(detail).getByText(
            "Idle observations record at least 120 seconds without observed activity. They do not prove that the agent finished.",
          ),
        ).toBeVisible()
      }
      expect(detail).not.toHaveTextContent("No assessment result is available yet.")
      if (clean > 0) {
        expect(within(detail).getByText("Saved assessment result.")).toBeVisible()
      } else if (finding > 0) {
        expect(within(detail).getByText("Review the recorded work.")).toBeVisible()
      }
    },
  )

  it.each(eventCases.flatMap((event) => [1, 3].map((count) => ({ ...event, count }))))(
    "keeps $count $event events in detail and leaves the row and header not assessed",
    async ({ event, count, singular, plural, limit }) => {
      setupWait(event, count)
      fireEvent.click(await screen.findByRole("button", { name: "Not assessed (1)" }))
      const label = count === 1 ? singular : plural
      const status = "Not assessed"
      const row = screen.getByRole("button", { name: `Failed retry loop, ${status}` })
      expect(row).toHaveTextContent(status)
      expect(row.querySelector(".activity-row-title-shimmer")).toBeNull()
      fireEvent.click(row)
      const detail = document.getElementById(`burn-check-${installed.effectiveId}-detail`)!
      expect(detail.querySelector("header")).toHaveTextContent(status)
      expect(
        within(detail).getByText(
          `This check is waiting for new ${plural}. It needs ${count} more ${label} before it can assess session activity.`,
        ),
      ).toBeVisible()
      if (limit) expect(within(detail).getByText(limit)).toBeVisible()
      if (event !== "toolResult") expect(detail).not.toHaveTextContent(/tool results?/)
      expect(screen.queryByRole("region", { name: "Checking 1" })).not.toBeInTheDocument()
    },
  )

  it.each(["userMessage", "turnEnd", "sessionIdle"] as const)(
    "hides the %s count until catch-up ends",
    async (event) => {
      setupWait(event, 3, true)
      fireEvent.click(await screen.findByRole("button", { name: "Not assessed (1)" }))
      fireEvent.click(
        screen.getByRole("button", {
          name: "Failed retry loop, Not assessed",
        }),
      )
      const detail = document.getElementById(`burn-check-${installed.effectiveId}-detail`)!
      expect(
        within(detail).getByText(
          "Session activity is still being processed. No assessment result is available yet.",
        ),
      ).toBeVisible()
      expect(detail).not.toHaveTextContent(/Waiting on|needs 3 more/)
    },
  )

  it("uses an assessment-scoped pass without a tool-result claim", async () => {
    setup(
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
            clean: 1,
            explanation: null,
            targets: [],
            waitingForResultsCount: 0,
            waitingForEvents: { event: "sessionIdle", count: 1, catchingUp: false },
          },
        ],
      },
    )
    fireEvent.click(
      await screen.findByRole("button", {
        name: "Failed retry loop, Passed",
      }),
    )
    expect(screen.getByText("No concern in this assessment.")).toBeVisible()
    expect(screen.queryByText(/No issue found in the checked results/)).not.toBeInTheDocument()
  })
})
