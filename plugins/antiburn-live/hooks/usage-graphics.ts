import { BITCOUNT_PIXEL, captionSvg, captionWidth } from './usage-font'
import { INLINE_METER_HEIGHT, METER_HEIGHT, PRESENTATION, meterGeometry, rollingColumns, type UsageLayout } from './usage-presentation'
import type { PreviewWarning } from './usage-fixtures'
import { pace, paceLabel, percentage, resetText, type UsageWindow } from './usage'

export { METER_HEIGHT }

export function xml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' })[c]!)
}

export function usageStatusSvg(text: string, hint?: string, compact = false): { source: string; width: number; height: number; description: string } {
  const hintX = compact ? 0 : captionWidth(text) + 12
  const width = Math.ceil(hint ? hintX + captionWidth(hint, 12) : captionWidth(text))
  const height = 18
  const description = hint ? `${text}. ${hint}` : text
  return {
    width,
    height,
    description,
    source: `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" preserveAspectRatio="xMinYMid meet"><title>${xml(description)}</title><defs><path id="bitcount-pixel" d="${BITCOUNT_PIXEL}"/></defs><style>.ink{fill:#242424}.hint{fill:#66615b}@media(prefers-color-scheme:dark){.ink{fill:#ececea}.hint{fill:#b8b5b0}}</style>${!compact || !hint ? `<g class="ink">${captionSvg(text, 0, 14)}</g>` : ''}${hint ? `<g class="hint">${captionSvg(hint, hintX, 13, 12)}</g>` : ''}</svg>`,
  }
}

export function meterDescription(window: UsageWindow, now: number): string {
  const value = pace(window, now)
  return `${window.label}: ${percentage(window)}. ${resetText(window, now)}${value === null ? '' : `. ${paceLabel(value)}`}`
}

export type MeterMotion = {
  layout?: UsageLayout
  warning?: PreviewWarning
  resetPreview?: 'show' | 'hide' | 'pace-show' | 'pace-hide'
  previousFillEnd?: number
  fillElapsed?: number
  entranceElapsed?: number
  meterIndex?: 0 | 1
  previousPercent?: string
  rollElapsed?: number
}

function fittedReset(text: string, width: number): string {
  if (captionWidth(text, 12, false, 0.025) <= width) return text
  const suffix = '...'
  if (captionWidth(suffix, 12, false, 0.025) > width) return ''
  let shortened = text
  while (shortened.length > 0 && captionWidth(shortened + suffix, 12, false, 0.025) > width) shortened = shortened.slice(0, -1)
  return shortened + suffix
}

function percentSvg(current: string, x: number, motion: MeterMotion, baseline: number = PRESENTATION.captionBaseline): string {
  if (motion.previousPercent === undefined || motion.rollElapsed === undefined || motion.rollElapsed >= 284) {
    return captionSvg(current, x, baseline, 15, true)
  }
  let left = x
  return rollingColumns(motion.previousPercent, current)
    .map((column, index) => {
      const at = left
      const width = captionWidth(column.character, 15, true)
      left += width
      const next = captionSvg(column.character, at, baseline, 15, true)
      if (!column.changed) return next
      const delay = column.delay - motion.rollElapsed!
      return (
        `<clipPath id="roll-${index}"><rect x="${at}" width="${width}" height="${motion.layout === 'inline' ? INLINE_METER_HEIGHT : PRESENTATION.captionHeight}"/></clipPath>` +
        `<g clip-path="url(#roll-${index})"><g class="roll-old" style="animation-delay:${delay}ms">${captionSvg(column.before, at, baseline, 15, true)}</g><g class="roll-new" style="animation-delay:${delay}ms">${next}</g></g>`
      )
    })
    .join('')
}

export function meterSvg(
  window: UsageWindow,
  width: number,
  now: number,
  revealed = false,
  caption = true,
  motion: MeterMotion = {},
): string {
  const inline = motion.layout === 'inline'
  const geometry = meterGeometry(window, width, now, motion.layout)
  const { dots, percent, percentX, resetX, resetWidth, fillEnd, marker, height, baseline, trackX, trackWidth, labelWidth } = geometry
  const resetBaseline = inline ? height / 2 + 3.6 : PRESENTATION.resetBaseline
  const labelHitWidth = inline ? labelWidth + 3 : Math.max(0, percentX - 8)
  const labelHitHeight = inline ? height : PRESENTATION.captionHeight
  const fillWidth = inline ? trackWidth : width
  const resetCopy =
    inline && resetWidth < 110 && window.resetsAt !== null ? resetText(window, now).replace(/^RESETS IN /, '') : resetText(window, now)
  const paceText = marker === null ? '' : `Even pace: ${Math.round(marker.value * 100)}%`
  const paceRight = marker === null ? 0 : Math.max(0, trackX + trackWidth - marker.x - 6)
  const paceLeft = marker === null ? 0 : Math.max(0, marker.x - trackX - 6)
  const paceOnRight = captionWidth(paceText, 12, false, 0.025) <= paceRight || paceRight >= paceLeft
  const paceCopy = fittedReset(paceText, paceOnRight ? paceRight : paceLeft)
  const paceCopyWidth = captionWidth(paceCopy, 12, false, 0.025)
  const paceCopyX = marker === null ? trackX : paceOnRight ? marker.x + 6 : marker.x - 6 - paceCopyWidth
  const trackTop = PRESENTATION.captionHeight
  const elapsed = motion.entranceElapsed
  const entering = elapsed !== undefined && elapsed < PRESENTATION.entranceEnd
  const delay = (motion.meterIndex === 1 ? 240 : 100) - (elapsed ?? 0)
  const animateFill = motion.previousFillEnd !== undefined && motion.fillElapsed !== undefined && motion.fillElapsed < 360 && !entering
  const neutral = dots.map((dot) => `<circle class="off" cx="${dot.x}" cy="${dot.y}" r="4.5"/>`).join('')
  const usedCircle = (dot: (typeof dots)[number], red = false) =>
    `<circle class="${red ? 'over-used' : 'used'}" cx="${dot.x}" cy="${dot.y}" r="4.5"/>`
  const filled = dots
    .filter((dot) => animateFill || dot.fraction === 1)
    .map((dot) => usedCircle(dot))
    .join('')
  const redAfterPace =
    motion.warning === 'red' && marker?.reached
      ? `<g id="over-pace">${dots
          .filter((dot) => (animateFill || dot.fraction === 1) && dot.x - 4.5 >= marker.x)
          .map((dot) => usedCircle(dot, true))
          .join('')}</g>`
      : ''
  const description = meterDescription(window, now)
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${caption ? height : 12}" viewBox="0 ${caption ? 0 : trackTop} ${width} ${caption ? height : 12}" aria-label="${xml(description)}">` +
    `<defs><path id="bitcount-pixel" d="${BITCOUNT_PIXEL}"/>` +
    `<clipPath id="meter-mask"><rect width="${width}" height="${height}"/></clipPath>` +
    `<clipPath id="reset-bounds"><rect x="${resetX}" width="${resetWidth}" height="${inline ? height : trackTop}"/></clipPath>` +
    `<clipPath id="fill-front"><rect class="fill-front" x="${trackX}" width="${animateFill ? fillWidth : Math.max(0, fillEnd - trackX)}" height="${height}"/></clipPath></defs>` +
    `<style>
.ink{fill:#252525}.off{fill:#d1cec9}.used{fill:${motion.warning === 'pulse' ? '#ed4545' : '#ff5914'}}.over-used{fill:#ed4545}.marker{color:#666;opacity:${revealed ? 1 : 0.4};transition:opacity 125ms cubic-bezier(.23,1,.32,1)}.detail-reveal{fill:#64645f;opacity:0;clip-path:inset(0 100% 0 0);transition:opacity 180ms cubic-bezier(.32,.72,0,1),clip-path 180ms cubic-bezier(.32,.72,0,1)}.reset{opacity:${revealed ? 1 : 0};clip-path:inset(0 ${revealed ? 0 : 100}% 0 0)}
@media(hover:hover) and (pointer:fine){${inline ? '.meter:has(.label:hover)' : '.meter:hover'} .reset,.meter:has(.pace:hover) .pace-detail{opacity:1;clip-path:inset(0);transition-duration:280ms;transition-delay:300ms}.meter:hover .marker{opacity:1;transition-delay:300ms}.meter:has(.pace:hover) .reset.detail-reveal{opacity:0;clip-path:inset(0 100% 0 0);transition-duration:280ms;transition-delay:300ms;animation:none}}
.meter:has(.label:focus) .reset,.meter:has(.pace:focus) .pace-detail{opacity:1;clip-path:inset(0);transition:none}.meter:has(.pace:focus) .reset{opacity:0;clip-path:inset(0 100% 0 0);transition:none;animation:none}.pace:focus .marker{opacity:1;transition:none}.hit{fill:transparent}.reset,.pace-detail{pointer-events:none}.reveal-background{fill:#f0efec}.detail-reveal .reveal-background{fill-opacity:.82}.focus-ring{fill:none;stroke:#ff5914;stroke-width:1;opacity:0}.label:focus .focus-ring,.pace:focus .focus-ring{opacity:1}
@media(prefers-color-scheme:dark){.ink{fill:#ececea}.off{fill:#4b4843}.marker{color:#b8b5b0}.reset,.pace-detail{fill:#b8b5b0}.reveal-background{fill:#212121}}
${motion.warning === 'pulse' ? '.debug-glow{opacity:.12;fill:#ed4545}.debug-fill{animation:debug-pulse 1600ms cubic-bezier(.77,0,.175,1) 6}@keyframes debug-pulse{0%,100%{opacity:1}50%{opacity:.65}}' : ''}
${motion.resetPreview ? `.${motion.resetPreview.startsWith('pace-') ? 'pace-detail' : 'reset'}{animation:debug-reset ${motion.resetPreview.endsWith('show') ? 280 : 180}ms cubic-bezier(.32,.72,0,1) ${motion.resetPreview.endsWith('hide') ? 'reverse' : 'normal'} both}@keyframes debug-reset{from{opacity:0;clip-path:inset(0 100% 0 0)}to{opacity:1;clip-path:inset(0)}}.meter:has(.label:focus) .reset{animation:none}` : ''}
${animateFill ? `.fill-front{transform-origin:${trackX}px 0;animation:preview-fill 360ms cubic-bezier(.23,1,.32,1) ${-motion.fillElapsed!}ms both}@keyframes preview-fill{from{transform:scaleX(${fillWidth > 0 ? Math.max(0, motion.previousFillEnd! - trackX) / fillWidth : 0})}to{transform:scaleX(${fillWidth > 0 ? Math.max(0, fillEnd - trackX) / fillWidth : 0})}}` : ''}
@keyframes meter-in{from{opacity:0;transform:translateY(var(--meter-rise,6px))}to{opacity:1;transform:none}}
@keyframes fill-in{from{transform:scaleX(0)}to{transform:scaleX(1)}}
@keyframes unmask-meter{from{clip-path:url(#meter-mask)}to{clip-path:none}}
@keyframes unmask-fill{from{clip-path:url(#fill-front)}to{clip-path:none}}
@keyframes roll-in{from{transform:translateY(18.75px)}to{transform:none}}
@keyframes roll-out{from{transform:translateY(0);visibility:visible}to{transform:translateY(-18.75px);visibility:hidden}}
.roll-old{animation:roll-out 260ms cubic-bezier(.32,.72,0,1) both}.roll-new{animation:roll-in 260ms cubic-bezier(.32,.72,0,1) both}
${entering ? `.meter{animation:unmask-meter 960ms steps(1,end) ${-elapsed}ms both}.sweep{animation:unmask-fill 960ms steps(1,end) ${-elapsed}ms both}.composition{animation:meter-in 360ms cubic-bezier(.32,.72,0,1) ${delay}ms both}.fill-front{animation:fill-in 360ms cubic-bezier(.23,1,.32,1) ${600 - elapsed}ms both;transform-origin:${trackX}px 0}` : ''}
@media(prefers-reduced-motion:reduce){.debug-fill,.debug-glow,.detail-reveal{animation:none!important}.meter,.sweep{animation:none;clip-path:none}${animateFill ? ' .sweep{clip-path:url(#fill-front)}' : ''}.composition{--meter-rise:0px;animation-duration:80ms;animation-delay:${-(elapsed ?? 0)}ms;transform:none}.fill-front{animation:none;transform:${animateFill ? `scaleX(${fillWidth > 0 ? Math.max(0, fillEnd - trackX) / fillWidth : 0})` : 'none'}}.roll-old{display:none}.roll-new{animation:none}.detail-reveal{clip-path:none;transition:opacity 80ms cubic-bezier(.32,.72,0,1)}${inline ? '.meter:has(.label:hover)' : '.meter:hover'} .reset,.meter:has(.pace:hover) .pace-detail{clip-path:none;transition-duration:80ms}.marker{transition-duration:80ms}.meter:has(.pace:hover) .reset.detail-reveal{clip-path:none;transition:opacity 80ms cubic-bezier(.32,.72,0,1);transition-delay:0ms}.meter:has(.label:focus) .reset,.meter:has(.pace:focus) .pace-detail,.pace:focus .marker{transition:none}${motion.resetPreview ? ` .${motion.resetPreview.startsWith('pace-') ? 'pace-detail' : 'reset'}{opacity:${motion.resetPreview.endsWith('show') ? 1 : 0};clip-path:none}` : ''}}
</style>` +
    `<g class="meter" clip-path="url(#meter-mask)"><rect class="meter-hit" width="${width}" height="${height}" fill="transparent" pointer-events="all"/><g class="composition">` +
    `<g class="label ink" tabindex="0" aria-label="${xml(`${window.label}, ${resetText(window, now)}`)}"><rect class="hit" width="${labelHitWidth}" height="${labelHitHeight}"/>` +
    `<g class="caption-ink">${captionSvg(window.label, 0, baseline)}</g>` +
    `<rect class="focus-ring" x=".5" y=".5" width="${Math.max(0, labelHitWidth - 1)}" height="${labelHitHeight - 1}"/></g>` +
    `<g class="ink data" aria-label="${xml(percent)}">${percentSvg(percent, percentX, motion, baseline)}</g>` +
    '<g class="data">' +
    neutral +
    (motion.warning === 'pulse'
      ? `<g class="debug-glow"><g class="debug-fill">${dots
          .filter((dot) => dot.fraction > 0)
          .map((dot) => `<circle cx="${dot.x}" cy="${dot.y}" r="6"/>`)
          .join('')}</g></g>`
      : '') +
    `<g class="sweep${motion.warning === 'pulse' ? ' debug-fill' : ''}"${entering || animateFill ? ' clip-path="url(#fill-front)"' : ''}>${filled}${redAfterPace}</g>` +
    '</g>' +
    (marker === null
      ? ''
      : `<g class="pace" tabindex="0" aria-label="${xml(paceLabel(marker.value))}"><rect class="hit" x="${Math.max(0, Math.min(width - 20, marker.x - 10))}" y="${inline ? 0 : Math.max(0, geometry.trackY - 12)}" width="20" height="${inline ? height : 24}"/><path class="marker" d="${marker.reached ? `M${marker.x} ${marker.y - marker.height / 2}v${marker.height}` : [-6, 0, 6].map((offset) => `M${marker.x} ${marker.y + offset - PRESENTATION.paceDash / 2}v${PRESENTATION.paceDash}`).join(' ')}" stroke="${marker.reached ? '#ff5914' : 'currentColor'}" stroke-width="2"/><rect class="focus-ring" x="${Math.max(0, Math.min(width - 20, marker.x - 10)) + 0.5}" y="${inline ? 0.5 : Math.max(0, geometry.trackY - 12) + 0.5}" width="19" height="${inline ? height - 1 : 23}"/></g>`) +
    `<g clip-path="url(#reset-bounds)"><g class="reset detail-reveal">${inline ? `<rect class="reveal-background" x="${resetX}" width="${resetWidth}" height="${height}"/>` : ''}${captionSvg(fittedReset(resetCopy, resetWidth), resetX, resetBaseline, 12, false, 0.025)}</g></g>` +
    (inline && marker !== null
      ? `<g class="pace-detail detail-reveal"><rect class="reveal-background" x="${trackX}" width="${trackWidth}" height="${height}"/>${captionSvg(paceCopy, paceCopyX, resetBaseline, 12, false, 0.025)}</g>`
      : '') +
    '</g></g></svg>'
  )
}

export function markSvg(source: string, elapsed?: number): string {
  if (elapsed === undefined || elapsed >= 960) return source
  const open = source.indexOf('>') + 1
  return (
    source.slice(0, open) +
    `<style>@keyframes mark-in{from{opacity:0;transform:scale(var(--mark-start,.92))}to{opacity:1;transform:none}}.mark{transform-origin:22.4px 22.1px;animation:mark-in 240ms cubic-bezier(.32,.72,0,1) ${-elapsed}ms both}@media(prefers-reduced-motion:reduce){.mark{--mark-start:1;animation-duration:80ms}}</style><g class="mark">` +
    source.slice(open).replace('</svg>', '</g></svg>')
  )
}

export function meterLayer(source: string, layer: 'base' | 'interaction'): string {
  const rules =
    layer === 'base' ? '.reset,.pace-detail,.hit,.focus-ring{display:none}' : '.data,.caption-ink{display:none}.marker{opacity:0}'
  return source.replace('</style>', `${rules}</style>`)
}
