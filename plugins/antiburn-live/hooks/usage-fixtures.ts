import { normalizeUsage, type UsageSnapshot } from './usage'

export type PreviewWarning = 'red' | 'pulse'
export const PREVIEW_PACE = [55, 50] as const
export const USAGE_FIXTURES = [
  { id: 'inactive', label: 'Inactive session', values: [null, 1] },
  { id: 'low', label: 'Low usage · 8% / 1%', values: [8, 1] },
  { id: 'mid', label: 'Mid usage · 48% / 36%', values: [48, 36] },
  { id: 'beyond-pace', label: 'Beyond pace · 68% / 58% · red', values: [68, 58], warning: 'red' },
  { id: 'high', label: 'High usage · 88% / 82% · red', values: [88, 82], warning: 'red' },
  { id: 'near-limit', label: 'Near limit · 98% / 96% · pulse + glow', values: [98, 96], warning: 'pulse' },
  { id: 'nine', label: 'Numeric roll · 9%', values: [9, 9] },
  { id: 'ten', label: 'Numeric roll · 10%', values: [10, 10] },
  { id: 'ninety-nine', label: 'Numeric roll · 99%', values: [99, 99] },
  { id: 'hundred', label: 'Numeric roll · 100%', values: [100, 100] },
  { id: 'empty', label: 'No reading yet', values: [] },
  { id: 'error', label: 'Read error · last known 48% / 36%', values: [48, 36] },
] as const

export function usageFixture(id: string, now: number): { snapshot: UsageSnapshot; warning?: PreviewWarning } | undefined {
  const fixture = USAGE_FIXTURES.find((candidate) => candidate.id === id)
  if (fixture === undefined) return undefined
  const snapshot = normalizeUsage(
    fixture.values.flatMap((value, index) =>
      value === null
        ? []
        : [
            {
              kind: index === 0 ? 'five_hour' : 'seven_day',
              percentUsed: value,
              resetsAt: new Date(now + (index === 0 ? 5 : 168) * 60 * 60_000 * (1 - PREVIEW_PACE[index]! / 100)).toISOString(),
            },
          ],
    ),
    now,
  )
  if (id === 'error') snapshot.status = 'error'
  return { snapshot, warning: 'warning' in fixture ? fixture.warning : undefined }
}
