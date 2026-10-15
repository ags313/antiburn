import type {
  ProviderUsageWindowPayload,
  ProviderUsageWindowsPayload,
} from "../../../lib/providerUsageIpc"
import {
  formatSpendFigure,
  formatTokenFigure,
  sessionCountLabel,
  windowTokens,
} from "../../../lib/presentation/providerUsage"

import { HeroFigures, type HeroFigureCell } from "../../../components/ui/HeroFigures"
import { SegmentFigure } from "../../../components/ui/SegmentFigure"
import { Skeleton } from "../../../components/ui/Skeleton"
import { useEntranceProps } from "./overviewEntrance"
import type { LocalUsageUnit } from "./overviewViewPrefs"

const SPANS: ReadonlyArray<{
  key: keyof ProviderUsageWindowsPayload
  label: string
  accessibleLabel: string
  span: string
}> = [
  { key: "today", label: "Today", accessibleLabel: "Today", span: "since midnight" },
  { key: "week", label: "7 days", accessibleLabel: "Last 7 days", span: "in the last 7 days" },
  {
    key: "last30Days",
    label: "30 days",
    accessibleLabel: "Last 30 days",
    span: "in the last 30 days",
  },
]

function spanTooltip(span: string, unit: LocalUsageUnit): string {
  return unit === "cost"
    ? `Estimated cost, at list price, of the tokens you used ${span}.`
    : `Input, output, and cache-read tokens you used ${span}.`
}

export function OverviewSpendTotals({
  totals,
  unit = "cost",
  loading = false,
}: {
  totals: ProviderUsageWindowsPayload | null
  unit?: LocalUsageUnit
  loading?: boolean
}) {
  const entranceProps = useEntranceProps(
    "spend-totals",
    "overview-figures-in",
    !loading && totals != null,
  )
  const cells: HeroFigureCell[] = SPANS.map((span) => ({
    key: span.key,
    label: (
      <>
        <span aria-hidden="true">{span.label}</span>
        <span className="sr-only">{span.accessibleLabel}</span>
      </>
    ),
    tooltip: spanTooltip(span.span, unit),
    // The placeholders wrap a sample of the line they stand in for, so they
    // take that line's own height and nothing below the section shifts as the
    // real figures land.
    ...(loading || !totals
      ? {
          figure: (
            <Skeleton className="w-28">
              <SegmentFigure>$0.00</SegmentFigure>
            </Skeleton>
          ),
          caption: <Skeleton className="w-36 max-w-full">0 tokens, 0 sessions</Skeleton>,
        }
      : unit === "cost"
        ? spendCell(totals[span.key])
        : tokenCell(totals[span.key])),
  }))

  return (
    <section
      aria-label={unit === "cost" ? "Estimated local spend" : "Local token usage"}
      aria-busy={loading || undefined}
      {...entranceProps}
    >
      <HeroFigures cells={cells} />
    </section>
  )
}

function spendCell(
  window: ProviderUsageWindowPayload,
): Pick<HeroFigureCell, "figure" | "caption"> {
  const hasCost = window.estimatedUsd != null
  const tokens = windowTokens(window)

  return {
    figure: (
      <SegmentFigure>
        {hasCost ? formatSpendFigure(window.estimatedUsd ?? 0) : formatTokenFigure(tokens)}
      </SegmentFigure>
    ),
    caption: (
      <>
        {sessionCountLabel(window.sessionCount)}
        {hasCost && (
          <>
            <span aria-hidden="true"> · </span>
            <SegmentFigure>{`${formatTokenFigure(tokens)} tokens`}</SegmentFigure>
          </>
        )}
        {!window.costComplete && (
          <>
            <span aria-hidden="true"> · </span>
            <span title="Some models in this period have no known price.">partial</span>
          </>
        )}
      </>
    ),
  }
}

/** The token unit leads with the token count and keeps the cost beside it. */
function tokenCell(
  window: ProviderUsageWindowPayload,
): Pick<HeroFigureCell, "figure" | "caption"> {
  const hasCost = window.estimatedUsd != null

  return {
    figure: <SegmentFigure>{formatTokenFigure(windowTokens(window))}</SegmentFigure>,
    caption: (
      <>
        {sessionCountLabel(window.sessionCount)}
        {hasCost && (
          <>
            <span aria-hidden="true"> · </span>
            <SegmentFigure>
              {`${window.costComplete ? "" : "at least "}${formatSpendFigure(window.estimatedUsd ?? 0)}`}
            </SegmentFigure>
          </>
        )}
      </>
    ),
  }
}
