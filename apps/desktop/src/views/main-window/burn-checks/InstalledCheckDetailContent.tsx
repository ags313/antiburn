import { useId, useState } from "react"
import {
  getInstalledCheckEvidence,
  type BurnCheckTargetEvidencePayload,
  type InstalledCheckTargetPayload,
} from "../../../lib/insightsIpc"
import type { InstalledCheckRow } from "../../../lib/presentation/checkRows"
import { CheckTargetHeading } from "./BurnCheckTargetDetail"
import { InstalledCheckEvidence } from "./InstalledCheckEvidence"
import { FailedSessions } from "./BurnCheckTargetPresentation"
import { PassedCheckDetail } from "./BurnCheckDetailBody"
import { InstalledCheckDiagnostics } from "./InstalledCheckDiagnostics"
import { CheckDetailPanel } from "./CheckDetailPanel"

export function InstalledCheckDetailContent({
  check,
  active,
  snoozed,
}: {
  check: InstalledCheckRow
  active: boolean
  snoozed: boolean
}) {
  const installed = check.installed
  const diagnostics = installed.diagnostics.filter(
    (diagnostic) => diagnostic.severity === "error",
  )
  const notices = installed.diagnostics.filter((diagnostic) => diagnostic.severity === "info")
  const showEmptyState =
    !snoozed &&
    installed.availability.status === "available" &&
    installed.diagnostics.length === 0 &&
    check.reviewFailed === 0 &&
    check.reviewedWithoutStatus === 0 &&
    check.finding === 0 &&
    check.clean === 0
  const waiting = installed.waitingForEvents
  let emptyMessage = "This check has not checked any sessions yet."
  if (check.checking) {
    emptyMessage = "This check is queued or running. No assessment result is available yet."
  } else if (waiting?.catchingUp || installed.waitingForDataCount > 0) {
    emptyMessage =
      "Session activity is still being processed. No assessment result is available yet."
  } else if (waiting && waiting.count > 0) {
    emptyMessage = `This check is waiting for new tool results. It needs ${waiting.count} more ${waiting.count === 1 ? "tool result" : "tool results"} before it can assess session activity.`
  }
  return (
    <div className="space-y-3">
      <InstalledCheckDiagnostics diagnostics={diagnostics} active={active} />
      {showEmptyState ? (
        <CheckDetailPanel variant="info">
          <p>{emptyMessage}</p>
        </CheckDetailPanel>
      ) : check.lifecycle == null ? null : check.lifecycle === "passing" ? (
        check.clean > 0 && (
          <PassedCheckDetail>
            {installed.explanation ?? "No issue found in the checked results."}
          </PassedCheckDetail>
        )
      ) : (
        <div className="burn-check-target-list">
          {installed.targets.map((target) => (
            <InstalledCheckTarget key={target.id} target={target} active={active} />
          ))}
          {installed.targetsTruncated && (
            <p className="type-callout text-label-secondary">
              More affected sessions are not shown.
            </p>
          )}
        </div>
      )}
      <InstalledCheckDiagnostics diagnostics={notices} active={active} />
    </div>
  )
}

function InstalledCheckTarget({
  target,
  active,
}: {
  target: InstalledCheckTargetPayload
  active: boolean
}) {
  const [open, setOpen] = useState(false)
  const evidenceId = useId()
  const [evidence, setEvidence] = useState<BurnCheckTargetEvidencePayload | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(false)
  const load = async () => {
    if (loading || !active) return
    setLoading(true)
    setError(false)
    try {
      setEvidence(await getInstalledCheckEvidence(target.id))
    } catch {
      setError(true)
    } finally {
      setLoading(false)
    }
  }
  return (
    <article className="burn-check-resource min-w-0">
      <CheckTargetHeading agent={target.agent} title={target.title}>
        <span className="min-w-0 wrap-anywhere">
          {target.projectName}
          {target.sourcePath ? ` (${target.sourcePath})` : ""}
        </span>
      </CheckTargetHeading>
      <div className="burn-check-resource-body">
        {target.observation && target.observation.trim() !== target.title.trim() && (
          <p className="mt-2 type-callout text-label-secondary">{target.observation}</p>
        )}
        {target.evidenceAvailable && (
          <section className="mt-2" aria-label="Source details">
            <button
              type="button"
              className="burn-check-action type-caption"
              aria-expanded={open}
              aria-controls={evidenceId}
              onClick={() => {
                setOpen(!open)
                if (!open && !evidence) void load()
              }}
            >
              {open ? "Hide source details" : "Show source details"}
            </button>
            {open && (
              <div id={evidenceId} className="mt-2 space-y-2" aria-busy={loading}>
                {loading && (
                  <p role="status" className="type-caption text-label-tertiary">
                    Loading source details…
                  </p>
                )}
                {error && (
                  <div>
                    <p role="alert" className="type-callout text-label-secondary">
                      Could not load source details.
                    </p>
                    <button
                      type="button"
                      className="burn-check-action mt-2 type-callout"
                      onClick={() => void load()}
                    >
                      Retry source details
                    </button>
                  </div>
                )}
                {evidence?.status === "unavailable" && (
                  <p role="status" className="type-callout text-label-secondary">
                    Source details are unavailable.
                  </p>
                )}
                {evidence?.status === "available" && (
                  <ol className="space-y-3">
                    {evidence.items.map((item) => (
                      <li key={item.reference} className="space-y-1">
                        <h4 className="type-callout font-medium text-label">
                          {item.sourceLabel}
                        </h4>
                        <InstalledCheckEvidence item={item} />
                      </li>
                    ))}
                  </ol>
                )}
              </div>
            )}
          </section>
        )}
        <FailedSessions samples={target.samples} active={active} />
      </div>
    </article>
  )
}
