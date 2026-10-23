import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(join(root, 'apps/desktop/package.json'))
const ts = require('typescript')
const { chromium } = require('@playwright/test')
const temporary = mkdtempSync(join(tmpdir(), 'antiburn-usage-layout-'))
const modules = [
  'usage',
  'usage-presentation',
  'usage-font',
  'usage-font-data',
  'brand',
  'usage-graphics',
]
let browser
try {
  for (const name of modules) {
    const folder = name === 'usage-graphics' ? 'release/hooks' : 'hooks'
    const source = readFileSync(
      join(root, `plugins/antiburn-live/${folder}/${name}.ts`),
      'utf8',
    )
    const output = ts
      .transpileModule(source, {
        compilerOptions: { target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.ESNext },
      })
      .outputText.replaceAll(/from '(\.\/[^']+)'/g, "from '$1.mjs'")
    writeFileSync(join(temporary, `${name}.mjs`), output)
  }
  const { responsiveMeterSvg, RESPONSIVE_FRAME_WIDTH } = await import(
    pathToFileURL(join(temporary, 'usage-graphics.mjs'))
  )
  const { meterGeometry, PRESENTATION } = await import(
    pathToFileURL(join(temporary, 'usage-presentation.mjs'))
  )
  const { filledDotCount } = await import(pathToFileURL(join(temporary, 'usage.mjs')))
  const now = Date.parse('2026-01-01T00:00:00Z')
  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1800, height: 400 } })
  let cases = 0
  for (const cell of [6, 8, 8.85, 10, 12]) {
    for (const columns of [24, 40, 59, 60, 61, 80, 120]) {
      for (const percent of [null, 0, 1, 2.5, 27, 55, 99.5, 100, 105]) {
        const window = {
          kind: 'five_hour',
          label: '5-hour',
          percent,
          resetsAt: '2026-01-01T02:48:00Z',
        }
        const svg = responsiveMeterSvg(window, now)
        assert.ok(svg.length < 131072)
        const width = columns * cell
        const inline = columns >= 60
        await page.setContent(
          `<style>*{box-sizing:border-box}body{margin:0}.row{display:flex;align-items:center;width:${width}px}.meters{display:flex;flex-direction:${inline ? 'row' : 'column'};gap:${inline ? 3 * cell : 1}px;flex:1 1 0;min-width:0}.slot{display:flex;flex:1 1 0;min-width:0;width:${inline ? '0' : '100%'}}iframe{border:0;display:block;max-width:100%;height:18px}</style><div class="row"><div style="flex:none;width:${(PRESENTATION.brandInset / 8) * cell}px"></div><div style="flex:none;width:${((PRESENTATION.markSlotSize + PRESENTATION.brandGap) / 8) * cell}px"></div><div class="meters"><div class="slot"><iframe width="${RESPONSIVE_FRAME_WIDTH}"></iframe></div><div class="slot"><iframe width="${RESPONSIVE_FRAME_WIDTH}"></iframe></div></div><div style="flex:none;width:${(PRESENTATION.brandInset / 8) * cell}px"></div></div>`,
        )
        await page.locator('iframe').evaluateAll((frames, source) => {
          for (const frame of frames) frame.srcdoc = source
        }, svg)
        for (const frame of page.frames().slice(1)) {
          await frame.waitForSelector('.percent')
          const geometry = await frame.evaluate(() => {
            const svg = document.querySelector('svg')
            const percent = document.querySelector('.percent')
            const track = document.querySelector('.track-width')
            const fill = document.querySelector('.fill-width')
            return {
              width: innerWidth,
              scale: svg.getScreenCTM().a,
              percentX: percent.getCTM().e,
              track: parseFloat(getComputedStyle(track).width),
              fill: parseFloat(getComputedStyle(fill).width),
            }
          })
          const expected = meterGeometry(window, geometry.width, now, 'inline')
          assert.equal(geometry.scale, 1, 'Glyphs and dots must retain their pixel size')
          assert.ok(
            Math.abs(geometry.percentX - expected.percentX) < 0.02,
            'Percentage must reach the actual slot edge',
          )
          if (expected.dots.length >= 2) {
            const trackWidth = (expected.dots.length - 1) * 11 + 7
            assert.ok(
              Math.abs(geometry.track - trackWidth) < 0.02,
              'Track must contain whole dots',
            )
            const used = filledDotCount(percent, expected.dots.length)
            assert.ok(
              Math.abs(geometry.fill - (used === 0 ? 0 : (used - 1) * 11 + 7)) < 0.02,
              'Fill must match whole-dot rounding',
            )
          }
        }
        const bounds = await page.locator('.slot').evaluateAll((slots) =>
          slots.map((slot) => {
            const r = slot.getBoundingClientRect()
            return { x: r.x, right: r.right, y: r.y }
          }),
        )
        assert.ok(
          Math.abs(width - bounds[1].right - (PRESENTATION.brandInset / 8) * cell) < 0.02,
          'No trailing cell-to-pixel gap',
        )
        assert.equal(bounds[0].y === bounds[1].y, inline)
        cases++
      }
    }
  }
  console.log(
    `Passed ${cases} synthetic Desktop layout cases across cell scales, breakpoints and readings.`,
  )
} finally {
  await browser?.close()
  rmSync(temporary, { recursive: true, force: true })
}
