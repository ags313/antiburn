import { expect, test } from 'claude-code/testing'
import { meterDescription, meterSvg, secondaryTextSvg, usageStatusSvg } from './usage-graphics'
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

test('secondary copy uses dim Bitcount glyphs with accessible full labels and bounded wrapping', () => {
  const lines = ['Account retry at 3:07pm GMT+10 · Refresh checks session.']
  const compact = secondaryTextSvg(lines, 192, 2)
  expect(compact.width).toBe(192)
  expect(compact.height).toBe(36)
  expect(compact.description).toBe(lines.join(' '))
  expect(compact.source).toContain('class="hint"')
  expect(compact.source).toContain('.hint{fill:#66615b}')
  expect(compact.source).toContain('.hint{fill:#b8b5b0}')
  expect(compact.source).toContain('bitcount-pixel')
  expect(compact.source).not.toContain('<text')
  expect(compact.source).not.toContain('animation')
  const wide = secondaryTextSvg(lines, 960, 2)
  expect(wide.height).toBe(18)
  expect(wide.description).toBe(lines[0])
  const wrapped = secondaryTextSvg(lines, 192, 8)
  expect(wrapped.height > compact.height).toBe(true)
  expect(secondaryTextSvg(['<retry & refresh>'], 192).source).toContain('&lt;retry &amp; refresh&gt;')
})

test('checking dots animate opacity without changing the caption, size, or reduced-motion reading', () => {
  const checking = usageStatusSvg('Checking usage...')
  expect(checking.description).toBe('Checking usage...')
  expect(checking.height).toBe(18)
  expect(checking.width).toBe(usageStatusSvg('Checking usage...', undefined, true).width)
  expect((checking.source.match(/class="checking-dot"/g) ?? []).length).toBe(3)
  expect(checking.source).toContain('animation-delay:0ms')
  expect(checking.source).toContain('animation-delay:80ms')
  expect(checking.source).toContain('animation-delay:160ms')
  expect(checking.source).toContain('@keyframes checking-dot{0%,100%{opacity:.35}50%{opacity:1}}')
  expect(checking.source).toContain('@media(prefers-reduced-motion:reduce){.checking-dot{animation:none;opacity:1}}')
  expect(checking.source).not.toContain('<script')
  expect(usageStatusSvg('Usage unavailable').source).not.toContain('@keyframes')
  expect(usageStatusSvg('Checking usage').source).not.toContain('@keyframes')
})


test('unrelated redraws retain the same meter source until visible geometry or copy changes', () => {
  const window = primaryWindow(normalizeUsage([{ kind: 'five_hour', percentUsed: 34, resetsAt: '2026-01-01T03:20:00Z' }], now), 'five_hour')
  const source = meterSvg(window, 288, now + 10_000)
  for (const offset of [10_001, 10_100, 11_000]) expect(meterSvg(window, 288, now + offset)).toBe(source)
  expect(meterSvg({ ...window, percent: 35 }, 288, now + 10_000)).not.toBe(source)
  expect(meterSvg(window, 300, now + 10_000)).not.toBe(source)
  expect(meterSvg(window, 288, now + 70_000)).not.toBe(source)
})
