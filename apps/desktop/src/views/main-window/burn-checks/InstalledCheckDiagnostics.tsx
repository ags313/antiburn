import { ChevronRight, ExternalLink } from "lucide-react"
import { useId, useRef, useState } from "react"

import {
  openInstalledCheckDefinition,
  type InstalledCheckDiagnosticPayload,
  type InstalledCheckDiagnosticSourcePayload,
} from "../../../lib/insightsIpc"
import { pluralize } from "../../../lib/presentation/text"
import { installedCheckDiagnosticKey } from "../../../lib/presentation/installedCheckDiagnostics"
import { CheckDetailPanel } from "./CheckDetailPanel"

function OpenCheckFile({
  source,
  active,
}: {
  source: InstalledCheckDiagnosticSourcePayload
  active: boolean
}) {
  const opening = useRef(false)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  return (
    <div className="space-y-2">
      <button
        type="button"
        className="burn-check-action inline-flex items-center gap-1 type-callout"
        disabled={pending || !active}
        aria-busy={pending}
        onClick={async () => {
          if (opening.current || !active) return
          opening.current = true
          setPending(true)
          setError(null)
          try {
            await openInstalledCheckDefinition(source.reference)
          } catch (error) {
            setError(
              typeof error === "string"
                ? error
                : "Could not open the check file. Check that it still exists and that an app can open YAML files, then try again.",
            )
          } finally {
            opening.current = false
            setPending(false)
          }
        }}
      >
        <ExternalLink size={14} className="shrink-0" aria-hidden="true" />
        {pending ? "Opening check file…" : "Open check file"}
      </button>
      {error && (
        <p role="alert" className="type-callout text-label-secondary">
          {error}
        </p>
      )}
    </div>
  )
}

function InstalledCheckDiagnostic({
  diagnostic,
  active,
}: {
  diagnostic: InstalledCheckDiagnosticPayload
  active: boolean
}) {
  const titleId = useId()
  return (
    <section aria-labelledby={titleId}>
      <CheckDetailPanel variant={diagnostic.severity}>
        {diagnostic.code === "input-shortened" ? (
          <details className="group">
            <summary
              id={titleId}
              className="flex cursor-pointer list-none items-center justify-between gap-2 type-callout [&::-webkit-details-marker]:hidden"
            >
              {diagnostic.title}
              <ChevronRight
                size={14}
                className="shrink-0 transition-transform duration-[var(--duration-fast)] group-open:rotate-90"
                aria-hidden="true"
              />
            </summary>
            <div className="mt-2 space-y-2">
              <DiagnosticContent diagnostic={diagnostic} active={active} />
            </div>
          </details>
        ) : (
          <div className="space-y-2">
            <h3 id={titleId} className="type-callout font-semibold! text-label">
              {diagnostic.title}
            </h3>
            <DiagnosticContent diagnostic={diagnostic} active={active} />
          </div>
        )}
      </CheckDetailPanel>
    </section>
  )
}

function DiagnosticContent({
  diagnostic,
  active,
}: {
  diagnostic: InstalledCheckDiagnosticPayload
  active: boolean
}) {
  return (
    <>
      <p className="whitespace-pre-wrap wrap-anywhere">{diagnostic.message}</p>
      <DiagnosticLocation source={diagnostic.source} active={active} />
      {(diagnostic.details.length > 0 || diagnostic.affectedSessionCount > 0) && (
        <details>
          <summary className="burn-check-action type-caption">Technical details</summary>
          <div className="mt-2 space-y-3">
            <DiagnosticDetails diagnostic={diagnostic} />
          </div>
        </details>
      )}
    </>
  )
}

function DiagnosticLocation({
  source,
  active,
}: {
  source: InstalledCheckDiagnosticPayload["source"]
  active: boolean
}) {
  return source ? (
    <>
      <p className="type-caption text-label-secondary wrap-anywhere">
        {source.path}
        {source.line != null ? ` · Line ${source.line}` : ""}
        {source.column != null ? `, column ${source.column}` : ""}
      </p>
      <OpenCheckFile key={source.reference} source={source} active={active} />
    </>
  ) : null
}

function DiagnosticDetails({ diagnostic }: { diagnostic: InstalledCheckDiagnosticPayload }) {
  return (
    <div className="space-y-2 type-caption">
      {diagnostic.affectedSessionCount > 0 && (
        <p className="text-label-tertiary">
          {diagnostic.affectedSessionCount} affected{" "}
          {pluralize(diagnostic.affectedSessionCount, "session", "sessions")}
        </p>
      )}
      <dl className="space-y-2">
        {diagnostic.details.map((detail, index) => (
          <div key={`${detail.label}:${index}`}>
            <dt className="text-label-secondary wrap-anywhere">{detail.label}</dt>
            <dd className="font-mono whitespace-pre-wrap text-label-tertiary wrap-anywhere">
              {detail.value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  )
}

function RelatedDiagnostic({
  diagnostic,
  active,
}: {
  diagnostic: InstalledCheckDiagnosticPayload
  active: boolean
}) {
  const titleId = useId()
  return (
    <section aria-labelledby={titleId} className="space-y-2">
      <h4 id={titleId} className="type-callout font-semibold! text-label">
        {diagnostic.title}
      </h4>
      <DiagnosticContent diagnostic={diagnostic} active={active} />
    </section>
  )
}

export function InstalledCheckDiagnostics({
  diagnostics,
  active,
}: {
  diagnostics: readonly InstalledCheckDiagnosticPayload[]
  active: boolean
}) {
  const [diagnostic, ...related] = diagnostics
  return diagnostic ? (
    <div className="space-y-2">
      <InstalledCheckDiagnostic
        key={installedCheckDiagnosticKey(diagnostic)}
        diagnostic={diagnostic}
        active={active}
      />
      {related.length > 0 && (
        <details>
          <summary className="burn-check-action type-callout">
            {related.length} more {pluralize(related.length, "issue", "issues")}
          </summary>
          <div className="mt-3 space-y-3">
            {related.map((issue) => (
              <RelatedDiagnostic
                key={installedCheckDiagnosticKey(issue)}
                diagnostic={issue}
                active={active}
              />
            ))}
          </div>
        </details>
      )}
    </div>
  ) : null
}
