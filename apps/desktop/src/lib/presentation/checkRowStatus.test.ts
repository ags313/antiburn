import { describe, expect, it } from "vitest"
import type { InstalledCheckReportPayload } from "../insightsIpc"
import { installedCheckFixture } from "../../views/main-window/burn-checks/tests/installedCheckFixture"
import { checkRows } from "./checkRows"
import { checkResultStatus, checkRowStatus } from "./checkRowStatus"

function status(overrides: Partial<InstalledCheckReportPayload> = {}, snoozed = false) {
  const [check] = checkRows({
    evidenceSettled: true,
    windowSessions: 1,
    pendingEvidence: 0,
    deferredEvidence: 0,
    estimatedTokenBurnBasisPoints: null,
    categories: [],
    installedChecks: [{ ...installedCheckFixture, ...overrides }],
  })
  return checkRowStatus(check!, snoozed)
}

describe("installed check status precedence", () => {
  it("appends the actual remaining count to the result", () => {
    expect(status().text).toBe("1 failed · 1 passed · Waiting on 1 tool result")
    expect(
      status({ waitingForEvents: { event: "toolResult", count: 3, catchingUp: false } }).text,
    ).toBe("1 failed · 1 passed · Waiting on 3 tool results")
  })

  it.each<{
    name: string
    overrides: Partial<InstalledCheckReportPayload>
    text: string
    kind: string
  }>([
    {
      name: "source catch-up",
      overrides: { waitingForEvents: { event: "toolResult", count: 3, catchingUp: true } },
      text: "1 failed · 1 passed · Catching up",
      kind: "waiting",
    },
    {
      name: "initial source data without a trigger count",
      overrides: { waitingForEvents: null, waitingForDataCount: 1 },
      text: "1 failed · 1 passed · Catching up",
      kind: "waiting",
    },
    {
      name: "source data before an otherwise ready trigger",
      overrides: { waitingForDataCount: 1 },
      text: "1 failed · 1 passed · Catching up",
      kind: "waiting",
    },
    {
      name: "queued work before catch-up and older errors",
      overrides: { checkingCount: 2, waitingForDataCount: 1, reviewFailed: 1 },
      text: "1 failed · 1 passed · Could not check · Checking",
      kind: "checking",
    },
    {
      name: "execution failure before catch-up",
      overrides: { reviewFailed: 1, waitingForDataCount: 1 },
      text: "1 failed · 1 passed · Could not check",
      kind: "neutral",
    },
    {
      name: "paused availability before stale queued work",
      overrides: { availability: { status: "paused" }, checkingCount: 1 },
      text: "Paused",
      kind: "neutral",
    },
    {
      name: "invalid definition without a diagnostic",
      overrides: { availability: { status: "invalid" } },
      text: "Check needs fixing",
      kind: "neutral",
    },
    {
      name: "unsupported definition",
      overrides: { availability: { status: "unsupported" } },
      text: "Not supported",
      kind: "neutral",
    },
    {
      name: "unassessed without trigger metadata",
      overrides: { finding: 0, clean: 0, hasStarted: false, waitingForEvents: null },
      text: "Not assessed",
      kind: "neutral",
    },
    {
      name: "retained failure without trigger metadata",
      overrides: { clean: 0, waitingForEvents: null },
      text: "1 failed",
      kind: "failed",
    },
    {
      name: "retained pass without trigger metadata",
      overrides: { finding: 0, waitingForEvents: null },
      text: "Passed",
      kind: "passed",
    },
  ])("handles $name without claiming a remaining count", ({ overrides, text, kind }) => {
    expect(status(overrides).text).toBe(text)
    expect(status(overrides).parts.at(-1)?.kind).toBe(kind)
  })

  it.each([
    { hasStarted: false, checkingCount: 1, clean: 0, text: "Passed · Checking" },
    { hasStarted: true, checkingCount: 0, clean: 0, text: "Passed" },
    { hasStarted: true, checkingCount: 1, clean: 2, text: "2 passed · Checking" },
  ])("keeps provisional passes and explicit counts: $text", ({ text, ...overrides }) => {
    expect(status({ finding: 0, waitingForEvents: null, ...overrides }).text).toBe(text)
  })

  it("keeps unstarted checks not assessed while waiting", () => {
    expect(status({ hasStarted: false, finding: 0, clean: 0 }).text).toBe(
      "Not assessed · Waiting on 1 tool result",
    )
  })

  it("shows specific diagnostics before catch-up and snooze before all work", () => {
    const overrides = {
      waitingForDataCount: 1,
      diagnostics: [
        {
          code: "input-too-large",
          severity: "error" as const,
          title: "Input is too large",
          message: "Select less input.",
          source: null,
          details: [],
          affectedSessionCount: 1,
        },
      ],
    }
    expect(status(overrides).text).toBe("1 failed · 1 passed · Input too large")
    expect(status({ ...overrides, checkingCount: 1 }, true)).toEqual({
      parts: [{ kind: "neutral", text: "Snoozed" }],
      text: "Snoozed",
    })
  })

  it.each([
    { finding: 1, clean: 0, result: "1 failed" },
    { finding: 0, clean: 2, result: "2 passed" },
  ])("keeps $result after a failed recheck", ({ result, ...counts }) => {
    expect(status({ ...counts, reviewFailed: 1, waitingForEvents: null }).text).toBe(
      `${result} · Could not check`,
    )
  })

  it("keeps built-in checking counts and verification wording", () => {
    expect(
      checkResultStatus({ finding: 0, clean: 0, checking: true, checkingCount: 2 }).text,
    ).toBe("2 checking")
    expect(
      checkResultStatus({ finding: 1, clean: 0, lifecycle: "awaitingVerification" }).text,
    ).toBe("1 failed · Verifying")
  })
})
