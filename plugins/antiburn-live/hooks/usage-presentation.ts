import { captionWidth } from './usage-font'
import { filledDotCount, pace, percentage, reachedPace, type UsageWindow } from './usage'

export const PRESENTATION = {
  markSize: 14,
  brandInset: 5,
  brandGap: 13,
  inlineGap: 24,
  stackedGap: 15,
  inlineWidth: 640,
  captionHeight: 18.75,
  captionBaseline: 12.975,
  resetBaseline: 12.075,
  trackHeight: 12,
  dotSize: 9,
  dotGap: 4,
  dotPitch: 13,
  paceHeight: 18,
  paceDash: 3,
  entranceEnd: 960,
  rollDuration: 260,
  rollStagger: 12,
  rollMaxStagger: 24,
} as const
export type UsageLayout = 'inline' | 'current'
export const INLINE_METER_HEIGHT = 18

export const METER_HEIGHT = PRESENTATION.captionHeight + (PRESENTATION.trackHeight + PRESENTATION.paceHeight) / 2

export function usageLayout(contentWidth: number, variant: UsageLayout = 'current') {
  const width = Math.max(1, Number.isFinite(contentWidth) ? contentWidth : 1)
  const inline = width >= PRESENTATION.inlineWidth
  const metersWidth = Math.max(1, width - PRESENTATION.brandInset * 2 - PRESENTATION.markSize - PRESENTATION.brandGap)
  const meterWidth = inline ? (metersWidth - PRESENTATION.inlineGap) / 2 : metersWidth
  const meterHeight = variant === 'inline' ? INLINE_METER_HEIGHT : METER_HEIGHT
  const stackedGap = variant === 'inline' ? 10 : PRESENTATION.stackedGap
  const height = inline ? meterHeight : meterHeight * 2 + stackedGap
  return {
    width,
    height,
    inline,
    meterWidth,
    meterHeight,
    stackedGap,
    mark: { x: PRESENTATION.brandInset, y: (height - PRESENTATION.markSize) / 2 },
    meters: [
      { x: 32, y: 0 },
      { x: inline ? 32 + meterWidth + PRESENTATION.inlineGap : 32, y: inline ? 0 : meterHeight + stackedGap },
    ],
  }
}

export function meterGeometry(window: UsageWindow, width: number, now: number, variant: UsageLayout = 'current') {
  const inline = variant === 'inline'
  const height = inline ? INLINE_METER_HEIGHT : METER_HEIGHT
  const baseline = inline ? height / 2 + PRESENTATION.dotSize / 2 : PRESENTATION.captionBaseline
  const percent = percentage(window)
  const percentX = Math.max(0, width - captionWidth(percent, 15, true))
  const labelWidth = captionWidth(window.label)
  const trackX = inline ? Math.max(labelWidth, captionWidth('5-hour'), captionWidth('Weekly')) + 7 : 0
  const trackWidth = inline ? Math.max(0, percentX - 7 - trackX) : width
  const trackY = inline ? height / 2 : PRESENTATION.captionHeight + 6
  const count = Math.max(0, Math.floor((trackWidth - PRESENTATION.dotSize) / PRESENTATION.dotPitch) + 1)
  const span = Math.max(0, (count - 1) * PRESENTATION.dotPitch)
  const used = filledDotCount(window.percent, count)
  const dots =
    count < 2
      ? []
      : Array.from({ length: count }, (_, index) => ({
          x: trackX + PRESENTATION.dotSize / 2 + index * PRESENTATION.dotPitch,
          y: trackY,
          fraction: index < used ? 1 : 0,
        }))
  const lastUsed = [...dots].reverse().find((dot) => dot.fraction > 0)
  const value = pace(window, now)
  const resetX = inline ? trackX : labelWidth + 8
  return {
    dots,
    height,
    baseline,
    trackX,
    trackWidth,
    trackY,
    labelWidth,
    percent,
    percentX,
    resetX,
    resetWidth: inline ? trackWidth : Math.max(0, percentX - 8 - resetX),
    fillEnd: lastUsed === undefined ? trackX : lastUsed.x - 4.5 + 9 * lastUsed.fraction,
    marker:
      value === null || dots.length === 0
        ? null
        : {
            x: trackX + 4.5 + span * value,
            y: trackY,
            height: PRESENTATION.paceHeight,
            value,
            reached: reachedPace(window, value),
          },
  }
}

function clamp(value: number): number {
  return Math.max(0, Math.min(1, value))
}

export function bezier(progress: number, x1: number, y1: number, x2: number, y2: number): number {
  const x = clamp(progress)
  if (x === 0 || x === 1) return x
  const at = (t: number, a: number, b: number) => 3 * (1 - t) ** 2 * t * a + 3 * (1 - t) * t ** 2 * b + t ** 3
  let low = 0
  let high = 1
  for (let i = 0; i < 30; i++) {
    const mid = (low + high) / 2
    if (at(mid, x1, x2) < x) low = mid
    else high = mid
  }
  return at((low + high) / 2, y1, y2)
}

export function entranceFrame(elapsed: number, reduced = false) {
  if (reduced) {
    const opacity = bezier(elapsed / 80, 0.32, 0.72, 0, 1)
    return { markOpacity: opacity, markScale: 1, meters: [0, 1].map(() => ({ opacity, rise: 0 })), fill: 1, done: elapsed >= 80 }
  }
  const mark = bezier(elapsed / 240, 0.32, 0.72, 0, 1)
  return {
    markOpacity: mark,
    markScale: 0.92 + 0.08 * mark,
    meters: [100, 240].map((start) => {
      const progress = bezier((elapsed - start) / 360, 0.32, 0.72, 0, 1)
      return { opacity: progress, rise: 6 * (1 - progress) }
    }),
    fill: bezier((elapsed - 600) / 360, 0.23, 1, 0.32, 1),
    done: elapsed >= PRESENTATION.entranceEnd,
  }
}

export function rollingColumns(previous: string, current: string) {
  const old = [...previous].reverse()
  return [...current].map((character, index) => {
    const column = current.length - index - 1
    const before = old[column] ?? ' '
    return {
      character,
      before,
      changed: /[0-9]/.test(character) && before !== character,
      delay: Math.min(PRESENTATION.rollMaxStagger, column * PRESENTATION.rollStagger),
    }
  })
}

export type EntranceState = { phase: 'pending' | 'running' | 'consumed'; generation: number; startedAt: number | null }

// The session boundary owns this state. Render subtrees do not grant another run.
export class EntranceOwner {
  state: EntranceState = { phase: 'consumed', generation: 0, startedAt: null }

  load(eligible: boolean): void {
    this.state = { phase: eligible ? 'pending' : 'consumed', generation: this.state.generation + 1, startedAt: null }
  }

  claim(now: number, visible: boolean, hasReading: boolean): boolean {
    if (this.state.phase !== 'pending' || !visible || !hasReading) return false
    this.state = { ...this.state, phase: 'running', startedAt: now }
    return true
  }

  cancel(): void {
    this.state = { phase: 'consumed', generation: this.state.generation + 1, startedAt: null }
  }

  accepts(generation: number): boolean {
    return this.state.phase === 'running' && this.state.generation === generation
  }
}
