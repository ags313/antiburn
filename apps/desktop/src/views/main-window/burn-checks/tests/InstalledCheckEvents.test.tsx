import { fireEvent, screen, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { InstalledCheckEvent } from "../../../../lib/insightsIpc"
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

function setupWait(event: InstalledCheckEvent, count: number, catchingUp = false) {
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
        },
      ],
    },
  )
}

describe("installed check event waits", () => {
  it.each(eventCases.flatMap((event) => [1, 3].map((count) => ({ ...event, count }))))(
    "shows $count $event events in the row, header, and detail",
    async ({ event, count, singular, plural, limit }) => {
      setupWait(event, count)
      fireEvent.click(await screen.findByRole("button", { name: "Not assessed (1)" }))
      const label = count === 1 ? singular : plural
      const status = `Not assessed · Waiting on ${count} ${label}`
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
          name: "Failed retry loop, Not assessed · Catching up",
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
        name: "Failed retry loop, Passed · Waiting on 1 idle observation",
      }),
    )
    expect(screen.getByText("No concern in this assessment.")).toBeVisible()
    expect(screen.queryByText(/No issue found in the checked results/)).not.toBeInTheDocument()
  })
})
