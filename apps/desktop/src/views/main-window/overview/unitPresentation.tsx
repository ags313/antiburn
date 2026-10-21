import type { ProviderUsageWindowPayload } from "../../../lib/providerUsageIpc"
import type { HeroFigureCell } from "../../../components/ui/HeroFigures"
import { SegmentFigure } from "../../../components/ui/SegmentFigure"
import {
  formatSpendFigure,
  formatTokenFigure,
  sessionCountLabel,
  windowTokens,
} from "../../../lib/presentation/providerUsage"
import type { LocalUsageUnit } from "./overviewViewPrefs"

type UnitPresentation = {
  value: (usage: ProviderUsageWindowPayload) => number
  label: (usage: ProviderUsageWindowPayload) => string
  otherFigure: (usage: ProviderUsageWindowPayload) => string
  axisLabel: (value: number, wholeGuides: boolean) => string
  chartLabel: string
  daysLabel: string
  totalsLabel: string
  bannerName: string
  incomplete: (usage: ProviderUsageWindowPayload) => boolean
  spanTooltip: (span: string) => string
  cell: (usage: ProviderUsageWindowPayload) => Pick<HeroFigureCell, "figure" | "caption">
}

export const UNIT_PRESENTATION: Record<LocalUsageUnit, UnitPresentation> = {
  cost: {
    value: (usage) => usage.estimatedUsd ?? 0,
    label: spendLabel,
    otherFigure: (usage) => formatTokenFigure(windowTokens(usage)),
    axisLabel: (value, wholeGuides) =>
      wholeGuides ? `$${value.toLocaleString("en-US")}` : formatSpendFigure(value),
    chartLabel: "Estimated spend by day",
    daysLabel: "Estimated spend for the past 30 days",
    totalsLabel: "Estimated local spend",
    bannerName: "spend",
    incomplete: (usage) => !usage.costComplete,
    spanTooltip: (span) => `Estimated cost, at list price, of the tokens you used ${span}.`,
    cell: spendCell,
  },
  tokens: {
    value: windowTokens,
    label: tokenLabel,
    otherFigure: spendLabel,
    axisLabel: formatTokenFigure,
    chartLabel: "Tokens by day",
    daysLabel: "Tokens for the past 30 days",
    totalsLabel: "Local token usage",
    bannerName: "tokens",
    incomplete: () => false,
    spanTooltip: (span) => `Input, output, and cache-read tokens you used ${span}.`,
    cell: tokenCell,
  },
}

function spendLabel(usage: ProviderUsageWindowPayload): string {
  if (usage.estimatedUsd != null) {
    return `${usage.costComplete ? "" : "at least "}${formatSpendFigure(usage.estimatedUsd)}`
  }
  return windowTokens(usage) > 0 ? "not priced" : "no sessions"
}

function tokenLabel(usage: ProviderUsageWindowPayload): string {
  const tokens = windowTokens(usage)
  return tokens > 0 ? `${formatTokenFigure(tokens)} tokens` : "no sessions"
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
