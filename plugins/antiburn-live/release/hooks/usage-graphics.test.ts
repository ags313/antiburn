import { expect, test } from 'claude-code/testing'
import { meterDescription, meterSvg } from './usage-graphics'
import { primaryWindow, normalizeUsage } from './usage'

const now = Date.parse('2026-01-01T00:00:00Z')
test('release graphics expose keyboard details without motion, previews or website hit regions', () => {
  const window = primaryWindow(normalizeUsage([{ kind: 'five_hour', percentUsed: 8, resetsAt: '2026-01-01T02:48:00Z' }], now), 'five_hour')
  const svg = meterSvg(window, 288, now)
  expect(svg).toContain('tabindex="0"')
  expect(svg).toContain('.label:focus')
  expect(svg).toContain('.pace:focus')
  expect(svg).toContain('RESETS IN 2H 48M')
  expect(svg).toContain('Even pace: 44% used by now')
  expect(svg).not.toContain('animation:')
  expect(svg).not.toContain('debug')
  expect(svg).not.toContain('<a ')
  expect(meterDescription({ ...window, percent: null }, now)).toContain('Unavailable')
})

test('release glyphs escape host labels and keep exceeded percentages exact', () => {
  const window = { kind: 'five_hour', label: '<host & label>', percent: 105, resetsAt: null }
  expect(meterSvg(window, 192, now)).toContain('&lt;host &amp; label&gt;')
  expect(meterDescription(window, now)).toContain('105%')
})

test('quota interaction presentation retains the existing theme, pace marker and reduced-motion behavior', () => {
  const window = primaryWindow(normalizeUsage([{ kind: 'five_hour', percentUsed: 94, resetsAt: '2026-01-01T02:48:00Z' }], now), 'five_hour')
  const svg = meterSvg(window, 288, now)
  expect(svg).toContain('stroke="#ff5914" stroke-width="2"')
  expect(svg).toContain('width="20"')
  expect(svg).toContain('prefers-reduced-motion:reduce')
  expect(svg).toContain('transition-delay:300ms')
  expect(svg).toContain('.meter:has(.pace:focus) .reset{opacity:0')
  expect(svg).toContain('clip-path="url(#reset-bounds)"')
  expect(svg).not.toContain('@keyframes')
})
