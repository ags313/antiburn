import { useState } from "react"
import {
  getInstalledCheckEvidence,
  type BurnCheckTargetEvidencePayload,
  type InstalledCheckTargetPayload,
} from "../../../lib/insightsIpc"
import type { InstalledCheckRow } from "../../../lib/presentation/checkRows"
import { CheckTargetHeading, EvidenceExcerpt } from "./BurnCheckTargetDetail"
import { FailedSessions } from "./BurnCheckTargetPresentation"
import { PassedCheckDetail } from "./BurnCheckDetailBody"

export function InstalledCheckDetailContent({
  check,
  active,
}: {
  check: InstalledCheckRow
  active: boolean
}) {
  const installed = check.installed
  if (check.lifecycle == null) {
    if (installed.reviewFailed > 0) {
      return (
        <p role="status" className="type-callout text-label-secondary">
          The check could not complete for {installed.reviewFailed}{" "}
          {installed.reviewFailed === 1 ? "session" : "sessions"}. New tool results may trigger
          another check.
        </p>
      )
    }
    if (installed.reviewedWithoutStatus > 0) {
      return (
        <p role="status" className="type-callout text-label-secondary">
          The check completed for {installed.reviewedWithoutStatus}{" "}
          {installed.reviewedWithoutStatus === 1 ? "session" : "sessions"} without changing
          status.
        </p>
      )
    }
    return (
      <p className="type-callout text-label-secondary">
        {installed.diagnostic ?? "This check has not checked any sessions yet."}
      </p>
    )
  }
  return (
    <>
      {installed.diagnostic && (
        <p className="mb-3 type-callout text-label-secondary">{installed.diagnostic}</p>
      )}
      {installed.reviewFailed > 0 && (
        <p role="status" className="mb-3 type-callout text-label-secondary">
          The check could not complete for {installed.reviewFailed}{" "}
          {installed.reviewFailed === 1 ? "session" : "sessions"}.
        </p>
      )}
      {installed.reviewedWithoutStatus > 0 && (
        <p role="status" className="mb-3 type-callout text-label-secondary">
          The latest check made no status change for {installed.reviewedWithoutStatus}{" "}
          {installed.reviewedWithoutStatus === 1 ? "session" : "sessions"}.
        </p>
      )}
      {check.lifecycle === "passing" ? (
        <PassedCheckDetail>
          {installed.explanation ??
            "No issue was found in the work checked. Other work may not have been checked."}
        </PassedCheckDetail>
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
    </>
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
  const [evidence, setEvidence] = useState<BurnCheckTargetEvidencePayload | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(false)
  const load = async () => {
    if (loading) return
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
        {target.observation && (
          <p className="mt-2 type-callout text-label-secondary">{target.observation}</p>
        )}
        {target.evidenceAvailable && (
          <section className="mt-2" aria-label="Source details">
            <button
              type="button"
              className="burn-check-action type-caption"
              aria-expanded={open}
              onClick={() => {
                setOpen(!open)
                if (!open && !evidence) void load()
              }}
            >
              {open ? "Hide source details" : "Show source details"}
            </button>
            {open && (
              <div className="mt-2 space-y-2" aria-busy={loading}>
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
                      Retry
                    </button>
                  </div>
                )}
                {evidence?.status === "unavailable" && (
                  <p role="status" className="type-callout text-label-secondary">
                    Source details could not be verified.
                  </p>
                )}
                {evidence?.status === "available" && (
                  <ol className="space-y-3">
                    {evidence.items.map((item) => (
                      <li key={item.reference} className="space-y-1">
                        <p className="type-callout font-medium text-label">
                          {item.sourceLabel}
                        </p>
                        <EvidenceExcerpt item={item} showMetadata={false} collapseLong />
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
