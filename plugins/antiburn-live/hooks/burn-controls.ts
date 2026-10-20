import { meterLayer } from './usage-graphics'
import type { usageLayout } from './usage-presentation'

type Layout = ReturnType<typeof usageLayout>

function meterControls(source: string, index: number, x: number, y: number): string {
  const prefix = `burn-meter-${index}-`
  return meterLayer(source, 'interaction')
    .replace(/id="([^"]+)"/g, (_, id: string) => `id="${prefix}${id}"`)
    .replace(/url\(#([^)]+)\)/g, (_, id: string) => `url(#${prefix}${id})`)
    .replace(/href="#([^"]+)"/g, (_, id: string) => `href="#${prefix}${id}"`)
    .replace('<svg ', `<svg id="burn-controls-${index}" x="${x}" y="${y}" `)
    .replace('<style>', `<style>@scope (#burn-controls-${index}) {`)
    .replace('</style>', '.composition{animation:none!important;transform:none!important;opacity:1}}</style>')
}

export function burnControlsSvg(layout: Layout, sources: readonly string[], _remainingMs: number): string {
  const { width, height } = layout
  const left = layout.meters[0]!.x
  const controlsWidth = Math.max(1, width - left)
  const meters = sources.map((source, index) => meterControls(source, index, layout.meters[index]!.x, layout.meters[index]!.y)).join('')
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${controlsWidth}" height="${height}" viewBox="${left} 0 ${controlsWidth} ${height}">${meters}</svg>`
}
