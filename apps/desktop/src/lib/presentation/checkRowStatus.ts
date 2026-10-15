import type { ChecksCategoryLifecycle } from "../insightsIpc"
import { isInstalledCheck, type CheckRow } from "./checkRows"
import { smartCheckReviewPresentation } from "./smartCheckReview"

type StatusKind = "failed" | "passed" | "checking" | "neutral"

export interface CheckStatusFacts {
  lifecycle?: ChecksCategoryLifecycle | null
  finding: number
  clean: number
  checking?: boolean
  checkingCount?: number
  reviewFailed?: number
  reviewedWithoutStatus?: number
  waitingForDataCount?: number
  waitingForResultsCount?: number
  terminalLabel?: string | null
}

export function checkResultStatus(facts: CheckStatusFacts, snoozed = false) {
  const parts: Array<{ kind: StatusKind; text: string }> = []
  const passed =
    !facts.terminalLabel &&
    facts.lifecycle === "passing" &&
    (facts.clean > 0 || !facts.checking)
  if (!passed && facts.finding > 0)
    parts.push({ kind: "failed", text: `${facts.finding} failed` })
  if (passed || facts.clean > 0)
    parts.push({
      kind: "passed",
      text: !passed && facts.finding > 0 ? `${facts.clean} passed` : "Passed",
    })
  if (facts.lifecycle === "awaitingVerification" && !snoozed)
    parts.push({ kind: "neutral", text: "Awaiting verification" })
  if (facts.checking && !snoozed) {
    const text = facts.checkingCount == null ? "Checking" : `${facts.checkingCount} checking`
    parts.push({ kind: "checking", text })
  }
  if ((facts.reviewFailed ?? 0) > 0)
    parts.push({
      kind: "neutral",
      text: facts.reviewFailed === 1 ? "Could not check" : `${facts.reviewFailed} check errors`,
    })
  if ((facts.reviewedWithoutStatus ?? 0) > 0)
    parts.push({ kind: "neutral", text: "No status change" })
  if ((facts.waitingForDataCount ?? 0) > 0)
    parts.push({ kind: "neutral", text: "Waiting for session data" })
  if ((facts.waitingForResultsCount ?? 0) > 0)
    parts.push({ kind: "neutral", text: "Waiting for tool results" })
  if (parts.length === 0 && facts.terminalLabel)
    parts.push({ kind: "neutral", text: facts.terminalLabel })
  if (snoozed) parts.push({ kind: "neutral", text: "Snoozed" })
  if (parts.length === 0) parts.push({ kind: "neutral", text: "Not assessed" })
  return { parts, text: parts.map((part) => part.text).join(" · ") }
}

export function checkStatusFacts(check: CheckRow): CheckStatusFacts {
  const review = smartCheckReviewPresentation(check)
  return {
    lifecycle:
      review.terminal && check.finding === 0 && check.clean === 0
        ? null
        : (check.lifecycle ?? null),
    finding: check.finding,
    clean: check.clean,
    ...(check.checkingCount == null ? {} : { checkingCount: check.checkingCount }),
    checking:
      check.checking === true &&
      check.lifecycle !== "awaitingVerification" &&
      check.reviewState == null,
    ...(isInstalledCheck(check)
      ? {
          reviewFailed: check.reviewFailed,
          reviewedWithoutStatus: check.reviewedWithoutStatus,
          waitingForDataCount: check.installed.waitingForDataCount,
          waitingForResultsCount: check.installed.waitingForResultsCount,
        }
      : {}),
    terminalLabel: review.terminalLabel,
  }
}

export function checkRowStatus(check: CheckRow, snoozed = false) {
  return checkResultStatus(checkStatusFacts(check), snoozed)
}
