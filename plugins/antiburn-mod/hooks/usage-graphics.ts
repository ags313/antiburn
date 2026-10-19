import { BITCOUNT_PIXEL, captionSvg, captionWidth } from './usage-font'
import { meterGeometry, PRESENTATION, desktopPercentage } from './usage-presentation'
import { pace, paceLabel, percentage, resetText, type UsageWindow } from './usage'

function xml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' })[c]!)
}

export function meterDescription(window: UsageWindow, now: number, desktop = false): string {
  const value = pace(window, now)
  const reading =
    window.percent === null
      ? 'Unavailable'
      : desktop
        ? desktopPercentage(window)
        : percentage(window)
  return `${window.label}: ${reading}. ${resetText(window, now)}${value === null ? '' : `. ${paceLabel(value)}`}`
}

function fitted(text: string, width: number): string {
  if (captionWidth(text, 12, false, 0.025) <= width) return text
  if (captionWidth('...', 12, false, 0.025) > width) return ''
  let shortened = text
  while (shortened.length && captionWidth(shortened + '...', 12, false, 0.025) > width) shortened = shortened.slice(0, -1)
  return shortened + '...'
}

export function meterSvg(window: UsageWindow, width: number, now: number): string {
  const scene = meterGeometry(window, width, now, 'inline')
  const { dots, height, baseline, percentX, resetX, resetWidth, labelWidth, trackX, trackWidth } = scene
  const marker = scene.marker === null ? null : { ...scene.marker, x: Math.round(scene.marker.x) }
  const description = meterDescription(window, now, true)
  const exhausted = window.percent !== null && window.percent >= 100
  const fill = exhausted ? '#ed4545' : '#ff5914'
  const circles = dots
    .map(
      (dot) =>
        `<circle class="${dot.fraction === 1 ? 'used' : 'off'}" cx="${dot.x}" cy="${dot.y}" r="${PRESENTATION.dotSize / 2}"/>`,
    )
    .join('')
  const resetCopy =
    resetWidth < 110 && window.resetsAt !== null ? resetText(window, now).replace(/^RESETS IN /, '') : resetText(window, now)
  const reset = fitted(resetCopy, resetWidth)
  const paceText = marker === null ? '' : `Even pace: ${Math.round(marker.value * 100)}%`
  const paceRight = marker === null ? 0 : Math.max(0, trackX + trackWidth - marker.x - 6)
  const paceLeft = marker === null ? 0 : Math.max(0, marker.x - trackX - 6)
  const paceOnRight = captionWidth(paceText, 12, false, 0.025) <= paceRight || paceRight >= paceLeft
  const paceCopy = fitted(paceText, paceOnRight ? paceRight : paceLeft)
  const paceCopyX = marker === null ? trackX : paceOnRight ? marker.x + 6 : marker.x - 6 - captionWidth(paceCopy, 12, false, 0.025)
  const markerX = marker === null ? 0 : Math.max(0, Math.min(width - 20, marker.x - 10))
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" aria-label="${xml(description)}">` +
    `<defs><path id="bitcount-pixel" d="${BITCOUNT_PIXEL}"/><clipPath id="reset-bounds"><rect x="${resetX}" width="${resetWidth}" height="${height}"/></clipPath></defs>` +
    `<style>
.ink{fill:#252525}.off{fill:#d1cec9}.used{fill:${fill};fill-opacity:${PRESENTATION.fillOpacity}}.marker{color:#666;opacity:.4;transition:opacity 125ms cubic-bezier(.23,1,.32,1)}.detail-reveal{fill:#64645f;opacity:0;clip-path:inset(0 100% 0 0);transition:opacity 180ms cubic-bezier(.32,.72,0,1),clip-path 180ms cubic-bezier(.32,.72,0,1)}
@media(hover:hover) and (pointer:fine){.meter:has(.label:hover) .reset,.meter:has(.pace:hover) .pace-detail{opacity:1;clip-path:inset(0);transition-duration:280ms;transition-delay:300ms}.meter:hover .marker{opacity:1;transition-delay:300ms}.meter:has(.pace:hover) .reset{opacity:0;clip-path:inset(0 100% 0 0)}}
.meter:has(.label:focus) .reset,.meter:has(.pace:focus) .pace-detail{opacity:1;clip-path:inset(0);transition:none}.meter:has(.pace:focus) .reset{opacity:0;clip-path:inset(0 100% 0 0);transition:none}.pace:focus .marker{opacity:1;transition:none}.hit{fill:transparent}.reset,.pace-detail{pointer-events:none}.reveal-background{fill:#f0efec;fill-opacity:.82}.focus-ring{fill:none;stroke:#ff5914;stroke-width:1;opacity:0}.label:focus .focus-ring,.pace:focus .focus-ring{opacity:1}
@media(prefers-color-scheme:dark){.ink{fill:#ececea}.off{fill:#4b4843}.marker{color:#b8b5b0}.reset,.pace-detail{fill:#b8b5b0}.reveal-background{fill:#212121}}
@media(prefers-reduced-motion:reduce){.detail-reveal{clip-path:none;transition:opacity 80ms cubic-bezier(.32,.72,0,1)}.meter:has(.label:hover) .reset,.meter:has(.pace:hover) .pace-detail{clip-path:none;transition-duration:80ms}.marker{transition-duration:80ms}.meter:has(.pace:hover) .reset{clip-path:none;transition:opacity 80ms cubic-bezier(.32,.72,0,1);transition-delay:0ms}.meter:has(.label:focus) .reset,.meter:has(.pace:focus) .pace-detail,.pace:focus .marker{transition:none}}
</style>` +
    `<g class="meter"><g class="label ink" tabindex="0" aria-label="${xml(`${window.label}, ${resetText(window, now)}`)}"><rect class="hit" width="${labelWidth + 3}" height="${height}"/>${captionSvg(window.label, 0, baseline, PRESENTATION.captionSize)}<rect class="focus-ring" x=".5" y=".5" width="${labelWidth + 2}" height="${height - 1}"/></g>` +
    `<g>${circles}</g><g class="ink">${captionSvg(desktopPercentage(window), percentX, baseline, PRESENTATION.captionSize, true)}</g>` +
    (marker === null
      ? ''
      : `<g class="pace" tabindex="0" aria-label="${xml(paceLabel(marker.value))}"><rect class="hit" x="${markerX}" width="20" height="${height}"/><path class="marker-shadow" d="M${marker.x + 1.5} ${marker.y - marker.height / 2}v${marker.height}" stroke="#000" stroke-opacity=".22" stroke-width="1" pointer-events="none"/><path class="marker" d="M${marker.x} ${marker.y - marker.height / 2}v${marker.height}" stroke="${exhausted ? '#ed4545' : marker.reached ? '#ff5914' : 'currentColor'}" stroke-width="2"/><rect class="focus-ring" x="${markerX + 0.5}" y=".5" width="19" height="${height - 1}"/></g>`) +
    `<g clip-path="url(#reset-bounds)"><g class="reset detail-reveal"><rect class="reveal-background" x="${resetX}" width="${resetWidth}" height="${height}"/>${captionSvg(reset, resetX, height / 2 + 3.6, 12, false, 0.025)}</g></g>` +
    (marker === null
      ? ''
      : `<g class="pace-detail detail-reveal"><rect class="reveal-background" x="${trackX}" width="${trackWidth}" height="${height}"/>${captionSvg(paceCopy, paceCopyX, height / 2 + 3.6, 12, false, 0.025)}</g>`) +
    '</g></svg>'
  )
}

export function usageStatusSvg(text: string, hint?: string, compact = false) {
  const hintX = compact ? 0 : captionWidth(text, PRESENTATION.captionSize) + 12
  const width = Math.ceil(
    hint ? hintX + captionWidth(hint, 12) : captionWidth(text, PRESENTATION.captionSize),
  )
  const description = hint ? `${text}. ${hint}` : text
  const checking = text === 'Checking usage...'
  const caption = checking
    ? captionSvg('Checking usage', 0, 13.2, PRESENTATION.captionSize) +
      [0, 1, 2]
        .map(
          (index) =>
            `<g class="checking-dot" style="animation-delay:${index * 80}ms">${captionSvg('.', captionWidth('Checking usage' + '.'.repeat(index), PRESENTATION.captionSize), 13.2, PRESENTATION.captionSize)}</g>`,
        )
        .join('')
    : captionSvg(text, 0, 13.2, PRESENTATION.captionSize)
  const motion = checking
    ? '.checking-dot{animation:checking-dot 1200ms linear infinite}@keyframes checking-dot{0%,100%{opacity:.35}50%{opacity:1}}@media(prefers-reduced-motion:reduce){.checking-dot{animation:none;opacity:1}}'
    : ''
  return {
    width,
    height: 18,
    description,
    source: `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="18" viewBox="0 0 ${width} 18" preserveAspectRatio="xMinYMid meet"><title>${xml(description)}</title><defs><path id="bitcount-pixel" d="${BITCOUNT_PIXEL}"/></defs><style>.ink{fill:#242424}.hint{fill:#66615b}@media(prefers-color-scheme:dark){.ink{fill:#ececea}.hint{fill:#b8b5b0}}${motion}</style>${!compact || !hint ? `<g class="ink">${caption}</g>` : ''}${hint ? `<g class="hint">${captionSvg(hint, hintX, 13, 12)}</g>` : ''}</svg>`,
  }
}

export function secondaryTextSvg(lines: readonly string[], availableWidth: number, maxLines = 2) {
  const width = Math.max(1, Math.floor(availableWidth))
  const wrapped = lines.flatMap((text) => {
    const rows: string[] = []
    let row = ''
    for (const word of text.split(/\s+/)) {
      const next = row ? `${row} ${word}` : word
      if (row && captionWidth(next, 12, false, 0.025) > width) {
        rows.push(fitted(row, width))
        row = word
      } else row = next
    }
    if (row) rows.push(fitted(row, width))
    return rows
  })
  const limit = Math.max(1, Math.floor(maxLines))
  const rows = wrapped.length <= limit ? wrapped : [...wrapped.slice(0, limit - 1), fitted(wrapped.slice(limit - 1).join(' '), width)]
  const height = Math.max(1, rows.length) * 18
  const description = lines.join(' ')
  return {
    width,
    height,
    description,
    source: `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><title>${xml(description)}</title><defs><path id="bitcount-pixel" d="${BITCOUNT_PIXEL}"/></defs><style>.hint{fill:#66615b}@media(prefers-color-scheme:dark){.hint{fill:#b8b5b0}}</style><g class="hint">${rows.map((line, index) => captionSvg(line, 0, index * 18 + 13, 12, false, 0.025)).join('')}</g></svg>`,
  }
}

export const MARK_DOTS: readonly (readonly [number, number])[] = [
  [16, 9.3],
  [22.4, 9.3],
  [28.8, 9.3],
  [35.2, 15.7],
  [16, 22.1],
  [22.4, 22.1],
  [28.8, 22.1],
  [35.2, 22.1],
  [9.6, 28.5],
  [35.2, 28.5],
  [16, 34.9],
  [22.4, 34.9],
  [35.2, 34.9],
]

export const BRAND_MARK_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="6.2 5.9 32.4 32.4">' +
  MARK_DOTS.map(([cx, cy]) => `<circle cx="${cx}" cy="${cy}" r="3.2" fill="#FF5914"/>`).join('') +
  '</svg>'
