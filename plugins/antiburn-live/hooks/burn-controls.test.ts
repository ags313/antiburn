import { test, expect } from 'claude-code/testing'
import { burnControlsSvg } from './burn-controls'
import { meterSvg } from './usage-graphics'
import { usageLayout } from './usage-presentation'

test('burn controls preserve reset and pace interactions outside the logo', () => {
  for (const width of [300, 800]) {
    for (const variant of ['inline', 'current'] as const) {
      const layout = usageLayout(width, variant)
      const sources = [
        { kind: 'five_hour', label: '5-hour', percent: 8, resetsAt: '2026-10-16T12:00:00Z' },
        { kind: 'seven_day', label: 'Weekly', percent: 1, resetsAt: '2026-10-21T12:00:00Z' },
      ].map((window) => meterSvg(window, layout.meterWidth, Date.parse('2026-10-16T10:00:00Z'), false, true, { layout: variant }))
      const svg = burnControlsSvg(layout, sources, 7500)
      expect(svg).toContain(`viewBox="32 0 ${width - 32} ${layout.height}"`)
      expect(svg).not.toContain('High output activity')
      expect(svg).not.toContain('burn-trigger')
      expect(svg).toContain('@scope (#burn-controls-0)')
      expect(svg).toContain('@scope (#burn-controls-1)')
      expect(svg).toContain('href="#burn-meter-0-bitcount-pixel"')
      expect(svg).toContain('href="#burn-meter-1-bitcount-pixel"')
      expect(svg).toContain('transition-delay:300ms')
      expect(svg).toContain('transition-duration:280ms')
      expect(svg).toContain('prefers-reduced-motion:reduce')
      expect(svg).toContain('transition:opacity 80ms')
      expect(svg).toContain('fill-opacity:.82')
      const ids = [...svg.matchAll(/id="([^"]+)"/g)].map((match) => match[1])
      expect(new Set(ids).size).toBe(ids.length)
      expect(svg.length < 131072).toBe(true)
      expect(svg).not.toContain('<title>')
      expect(svg).not.toContain('<script')
    }
  }
})

test('empty burn controls contain no logo hit target', () => {
  const layout = { ...usageLayout(300, 'inline'), height: 20, mark: { x: 5 as const, y: 3 } }
  const svg = burnControlsSvg(layout, [], 0)
  expect(svg).toContain('height="20" viewBox="32 0 268 20"')
  expect(svg).not.toContain('burn-trigger')
  expect(svg).not.toContain('burn-controls-0')
})
