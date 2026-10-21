import type { ProviderUsageWindowsPayload } from "../../../lib/providerUsageIpc"

import { HeroFigures, type HeroFigureCell } from "../../../components/ui/HeroFigures"
import { SegmentFigure } from "../../../components/ui/SegmentFigure"
import { Skeleton } from "../../../components/ui/Skeleton"
import { useEntranceProps } from "./overviewEntrance"
import type { LocalUsageUnit } from "./overviewViewPrefs"
import { UNIT_PRESENTATION } from "./unitPresentation"

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

export function OverviewUsageTotals({
  totals,
  unit = "cost",
  loading = false,
}: {
  totals: ProviderUsageWindowsPayload | null
  unit?: LocalUsageUnit
  loading?: boolean
}) {
  const presentation = UNIT_PRESENTATION[unit]
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
    tooltip: presentation.spanTooltip(span.span),
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
      : presentation.cell(totals[span.key])),
  }))

  return (
    <section
      aria-label={presentation.totalsLabel}
      aria-busy={loading || undefined}
      {...entranceProps}
    >
      <HeroFigures cells={cells} />
    </section>
  )
}
