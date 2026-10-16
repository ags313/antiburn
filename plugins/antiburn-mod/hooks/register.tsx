import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, RenderInput } from 'claude-code'
import type { UsageState } from '../types'
import { APP_URL } from './brand'
import {
  filledDotCount,
  normalizeUsage,
  pace,
  paceLabel,
  percentage,
  primaryWindow,
  resetText,
  terminalDotCount,
  usageFailure,
  UsageOrder,
  PRIMARY_WINDOWS,
  type UsageSnapshot,
  type UsageWindow,
} from './usage'
import { BRAND_MARK_SVG, meterDescription, meterSvg, usageStatusSvg } from './usage-graphics'
import { PRESENTATION, usageLayout } from './usage-presentation'

const BRAND = '#FF5914'
const usage = atom(
  { plugin: 'antiburn-mod', key: 'usage' } as const,
  { snapshot: { status: 'empty', windows: [], capturedAt: 0 }, revealed: null, activated: null, hovered: null } as UsageState,
)
const usageOrder = new UsageOrder()
const hoverSites = new Map<string, { shape: string; binding: string }>()
const hoverMount = Math.random().toString(36).slice(2)
let hoverRevision = 0
let commandName: string | undefined
let needsUsageReconciliation = false

function terminalHoverBinding(requestId: string, shape: string): string {
  const previous = hoverSites.get(requestId)
  if (previous?.shape === shape) return previous.binding
  const binding = `${hoverMount}:${++hoverRevision}`
  if (hoverSites.size >= 8 && !hoverSites.has(requestId)) hoverSites.delete(hoverSites.keys().next().value!)
  hoverSites.set(requestId, { shape, binding })
  return binding
}

async function commitUsage($: EngineInterface, snapshot: UsageSnapshot, ticket: { generation: number; revision: number }): Promise<void> {
  if (!usageOrder.isCurrent(ticket)) return
  await update($, usage, (current) => (usageOrder.isCurrent(ticket) ? { ...current, snapshot } : current))
}

async function readUsage($: EngineInterface): Promise<void> {
  const ticket = usageOrder.beginRead()
  try {
    const [reading, now] = await Promise.all([$.session.usage(), $.clock.now()])
    await commitUsage($, normalizeUsage(reading.rateLimits, now), ticket)
  } catch {
    if (!usageOrder.isCurrent(ticket)) return
    await update($, usage, (current) =>
      usageOrder.isCurrent(ticket) ? { ...current, snapshot: usageFailure(current.snapshot, current.snapshot.capturedAt) } : current,
    )
  }
}

async function resetUsage($: EngineInterface): Promise<void> {
  usageOrder.reset()
  hoverSites.clear()
  await update($, usage, () => ({
    snapshot: { status: 'empty' as const, windows: [], capturedAt: 0 },
    revealed: null,
    activated: null,
    hovered: null,
  }))
}

async function reconcileUsage($: EngineInterface): Promise<void> {
  if (!needsUsageReconciliation) return
  needsUsageReconciliation = false
  await readUsage($)
}

async function revealUsage($: EngineInterface, kind: string): Promise<void> {
  await update($, usage, (current) => {
    const dismissed = current.activated === kind
    return { ...current, revealed: dismissed ? null : kind, activated: dismissed ? null : kind }
  })
}

async function openTerminalWebsite($: EngineInterface): Promise<void> {
  try {
    const windows = (await $.env.get('OS')) === 'Windows_NT'
    const commands = windows
      ? [['cmd.exe', '/c', 'start', '', APP_URL]]
      : [
          ['open', APP_URL],
          ['xdg-open', APP_URL],
        ]
    for (const argv of commands) {
      try {
        const result = await $.process.run(argv, { timeoutMs: 5000 })
        if (result.exitCode === 0) return
      } catch {
        // Try the next system opener if this command fails.
      }
    }
  } catch {
    // Show the address if the host cannot identify the system.
  }
  await $.ui.status(`Open ${APP_URL} in your browser`)
}

async function renderUsage($: EngineInterface, e: RenderInput<'AbovePrompt'>) {
  const ui = $.ui.resolve(e)
  const { Box, Text, Button, Link } = ui
  const state = await read($, usage)
  const snapshot = state.snapshot
  const now = await $.clock.now()
  const windows = PRIMARY_WINDOWS.map((kind) => primaryWindow(snapshot, kind))
  const columns = Math.max(1, Math.min(4096, e.props.bodyColumns))
  const isDesktop = e.surface === 'desktop' && 'Svg' in ui
  const mark = isDesktop ? (
    <Box key="usage-mark-slot" position="relative" width={PRESENTATION.markSize / 8} flexShrink={0}>
      <ui.Svg key="usage-mark-size" source={BRAND_MARK_SVG} alt="antiburn" width={PRESENTATION.markSize} height={PRESENTATION.markSize} />
      <Box
        key="usage-brand-link"
        position="absolute"
        left={0}
        top={0}
        width={PRESENTATION.markSize / 8}
        height={PRESENTATION.markSize / 16}
        overflow="hidden"
      >
        <Link href={APP_URL} label="Open antiburn website">
          <Text>{'\u2003'}</Text>
        </Link>
      </Box>
    </Box>
  ) : (
    <Box
      key="usage-brand-text-slot"
      width={11}
      paddingLeft={1}
      flexDirection="row"
      flexShrink={0}
      height={2}
      alignItems="center"
      justifyContent="center"
    >
      <Box flexDirection="column" width={10} height={2} flexShrink={0}>
        <Text> </Text>
        <Box width={10} height={1} flexShrink={0} overflow="hidden">
          <Button key="usage-brand-open" plain label="Open antiburn website" onPress={() => openTerminalWebsite($)}>
            <Text color={BRAND} backgroundColor="#4D2F23">
              {' antiburn '}
            </Text>
          </Button>
        </Box>
      </Box>
    </Box>
  )
  const hasReading = windows.some((window) => window.percent !== null)
  if (!hasReading && snapshot.status !== 'ready') {
    terminalHoverBinding(e.requestId, 'empty')
    const status = snapshot.status === 'error' ? 'Usage unavailable' : 'Waiting for usage data'
    const canRefresh = columns >= 24
    const graphic = usageStatusSvg(status, undefined, columns < 60)
    return (
      <Box
        key="usage-band"
        flexDirection="row"
        alignItems={isDesktop ? 'center' : 'flex-end'}
        gap={isDesktop ? 0 : 1}
        height={isDesktop && columns >= 24 ? Math.max(graphic.height, usageLayout(columns * 8, 'inline').height) / 16 : undefined}
      >
        {isDesktop && <Box width={PRESENTATION.brandInset / 8} flexShrink={0} />}
        {mark}
        {isDesktop && <Box width={PRESENTATION.brandGap / 8} flexShrink={0} />}
        {isDesktop ? (
          <ui.Svg
            key="usage-status"
            source={graphic.source}
            alt={graphic.description}
            width={Math.min(
              graphic.width,
              Math.max(1, columns * 8 - PRESENTATION.brandInset - PRESENTATION.markSize - PRESENTATION.brandGap - (canRefresh ? 72 : 0)),
            )}
            height={graphic.height}
          />
        ) : (
          <Box width={Math.max(1, columns - (canRefresh ? 20 : 12))} flexShrink={1}>
            <Text key="usage-status" wrap="truncate-end">
              {status}
            </Text>
          </Box>
        )}
        {canRefresh && (
          <Button key="usage-refresh" plain label="Refresh usage" onPress={() => readUsage($)}>
            <Text>Refresh</Text>
          </Button>
        )}
      </Box>
    )
  }
  if (isDesktop && columns >= 24) {
    terminalHoverBinding(e.requestId, 'desktop')
    const layout = usageLayout(columns * 8, 'inline')
    return (
      <Box key="usage-band" position="relative" flexDirection="row" alignItems="center">
        <Box width={PRESENTATION.brandInset / 8} flexShrink={0} />
        {mark}
        <Box width={PRESENTATION.brandGap / 8} flexShrink={0} />
        <Box key="usage-meters" flexDirection={layout.inline ? 'row' : 'column'} gap={layout.inline ? 3 : layout.stackedGap / 16}>
          {windows.map((window) => (
            <Box key={`meter-${window.kind}`} height={layout.meterHeight / 16} alignItems="center">
              <ui.Svg
                key={`meter-graphic-${window.kind}`}
                source={meterSvg(window, layout.meterWidth, now)}
                alt={`${meterDescription(window, now)}${snapshot.status === 'error' ? '. Usage unavailable · Last known' : ''}`}
                width={layout.meterWidth}
                height={layout.meterHeight}
                isInteractive
              />
            </Box>
          ))}
        </Box>
        <Box width={PRESENTATION.brandInset / 8} flexShrink={0} />
      </Box>
    )
  }
  const content = Math.max(1, columns - (isDesktop ? 3 : 12))
  const inlineColumns = Math.max(1, Math.floor((content - 3) / 2))
  const isInline = columns >= 80 && windows.every((window) => window.label.length + percentage(window).length + 2 <= inlineColumns)
  const meterColumns = isInline ? inlineColumns : content
  const geometry = (window: UsageWindow) => {
    const trackColumns = Math.max(0, meterColumns - window.label.length - percentage(window).length - 2)
    const count = isDesktop ? 0 : terminalDotCount(trackColumns)
    const value = pace(window, now)
    const markerCell = value === null || count < 2 ? null : Math.max(1, Math.min(count * 2 - 3, Math.round(value * (count - 1)) * 2 - 1))
    return { count, value, markerCell }
  }
  const hoverBinding =
    e.surface === 'terminal'
      ? terminalHoverBinding(
          e.requestId,
          JSON.stringify([
            columns,
            snapshot.status,
            windows.map((window) => [window.resetsAt, percentage(window).length, geometry(window).markerCell]),
          ]),
        )
      : ''
  const revealed =
    e.surface === 'terminal' && state.hovered?.requestId === e.requestId && state.hovered.binding === hoverBinding
      ? state.hovered.target
      : state.revealed

  const meter = (window: UsageWindow) => {
    const description = meterDescription(window, now)
    const isRevealed = revealed === window.kind
    const isPaceRevealed = revealed === `pace-${window.kind}`
    const { count, value, markerCell } = geometry(window)
    const isPaceReached = window.percent !== null && value !== null && window.percent / 100 >= value
    const cells = count * 2 - 1
    const copy = isRevealed ? resetText(window, now) : isPaceRevealed ? (value === null ? 'Pace unavailable' : paceLabel(value)) : null
    const copyOnRight = markerCell !== null && cells - markerCell - 1 >= markerCell
    const copyOnLeft = isRevealed || (isPaceRevealed && !copyOnRight)
    const segment = (start: number, end: number, showCopy: boolean) => {
      const copyLength = showCopy && copy !== null ? Math.min(copy.length, Math.max(0, end - start)) : 0
      const copyStart = copyOnLeft ? end - copyLength : start
      const dotCells = Array.from({ length: Math.max(0, end - start) }, (_, index) => {
        const cell = start + index
        if (copyLength > 0 && cell >= copyStart && cell < copyStart + copyLength) return null
        if (cell % 2 !== 0) return <Text key={`space-${cell}`}> </Text>
        const filled = cell / 2 < filledDotCount(window.percent, count)
        return (
          <Text key={`dot-${cell}`} color={filled ? BRAND : 'subtle'}>
            {'●'}
          </Text>
        )
      })
      if (copyLength === 0) return dotCells
      dotCells.splice(
        copyStart - start,
        copyLength,
        <Text key="usage-segment-copy" dimColor wrap="truncate-end">
          {copy}
        </Text>,
      )
      return dotCells
    }

    const target = (detail: 'reset' | 'pace', text: string, color?: string, dimColor?: boolean) =>
      e.surface === 'terminal' && 'Client' in ui ? (
        <ui.Client
          key={`usage-hover-${detail}-${window.kind}`}
          module="./usage-target.tsx"
          width={detail === 'reset' ? window.label.length : 1}
          height={1}
          props={{
            binding: hoverBinding,
            width: detail === 'reset' ? window.label.length : 1,
            text,
            ...(color === undefined ? {} : { color }),
            ...(dimColor === undefined ? {} : { dimColor }),
          }}
        />
      ) : null
    const markerGlyph = isPaceReached ? '│' : '┊'
    const markerHover = target('pace', markerGlyph, isPaceReached ? BRAND : 'subtle', !isPaceReached && !isPaceRevealed)
    const paceMarker =
      markerCell !== null ? (
        <Box position="relative" width={1} height={1} flexShrink={0}>
          <Button key={`usage-pace-${window.kind}`} plain onPress={() => revealUsage($, `pace-${window.kind}`)}>
            <Text key="pace-marker" color={isPaceReached ? BRAND : 'subtle'} dimColor={!isPaceReached && !isPaceRevealed}>
              {isPaceReached ? '│' : '┊'}
            </Text>
          </Button>
          {markerHover && (
            <Box position="absolute" left={0} top={0} width={1} height={1}>
              {markerHover}
            </Box>
          )}
        </Box>
      ) : null
    const dots =
      count >= 2 ? (
        isRevealed && markerCell !== null ? (
          <Box key="usage-reset-track" position="relative" width={cells} height={1} overflow="hidden">
            <Text>{segment(0, cells, false)}</Text>
            <Box key="usage-pace-marker-slot" position="absolute" left={markerCell} top={0} width={1} height={1}>
              <Button key={`usage-pace-${window.kind}`} plain onPress={() => revealUsage($, `pace-${window.kind}`)}>
                <Text color={isPaceReached ? BRAND : 'subtle'}>{markerGlyph}</Text>
              </Button>
            </Box>
            <Box key="usage-reset-copy-track" position="absolute" left={0} top={0} width={cells} height={1} overflow="hidden">
              <Text key="usage-reset-copy" dimColor wrap="truncate-end">
                {copy}
              </Text>
            </Box>
            {markerHover && (
              <Box position="absolute" left={markerCell} top={0} width={1} height={1}>
                {target(
                  'pace',
                  copy !== null && markerCell < copy.length ? copy[markerCell]! : markerGlyph,
                  copy !== null && markerCell < copy.length ? undefined : isPaceReached ? BRAND : 'subtle',
                  copy !== null && markerCell < copy.length ? true : !isPaceReached,
                )}
              </Box>
            )}
          </Box>
        ) : (
          <Box flexDirection="row" width={cells} height={1} flexShrink={0}>
            {markerCell === null && <Text>{segment(0, cells, copy !== null)}</Text>}
            {markerCell !== null && (
              <Box
                key={`usage-pace-left-${window.kind}`}
                width={markerCell}
                flexShrink={0}
                justifyContent={isPaceRevealed && copyOnLeft ? 'flex-end' : undefined}
                height={1}
                flexDirection="row"
                overflow="hidden"
              >
                <Text wrap="truncate-end">{segment(0, markerCell, copy !== null && copyOnLeft)}</Text>
              </Box>
            )}
            {paceMarker}
            {markerCell !== null && (
              <Box
                key={`usage-pace-right-${window.kind}`}
                width={cells - markerCell - 1}
                flexShrink={0}
                height={1}
                flexDirection="row"
                overflow="hidden"
              >
                <Text wrap="truncate-end">{segment(markerCell + 1, cells, copy !== null && isPaceRevealed && copyOnRight)}</Text>
              </Box>
            )}
          </Box>
        )
      ) : null
    return (
      <Box key={`usage-${window.kind}`} position="relative" flexDirection="column" width={meterColumns} height={1}>
        <Box flexDirection="row" justifyContent="space-between" gap={1}>
          <Box position="relative" width={window.label.length} height={1} flexShrink={0} overflow="hidden">
            <Button key={`usage-detail-${window.kind}`} plain label={description} onPress={() => revealUsage($, window.kind)}>
              <Text>{window.label}</Text>
            </Button>
            {e.surface === 'terminal' && (
              <Box position="absolute" left={0} top={0} width={window.label.length} height={1}>
                {target('reset', window.label)}
              </Box>
            )}
          </Box>
          <Box flexGrow={1} flexShrink={1} minWidth={0}>
            {dots}
          </Box>
          <Text>{percentage(window)}</Text>
        </Box>
      </Box>
    )
  }

  const meters = (
    <Box key="usage-meters" flexDirection={isInline ? 'row' : 'column'} gap={isInline ? 3 : 0}>
      {windows.map(meter)}
    </Box>
  )
  if (snapshot.status === 'error') {
    return (
      <Box key="usage-band" flexDirection="column">
        <Box flexDirection="row" alignItems={isDesktop ? 'center' : 'flex-end'} gap={isDesktop ? 0 : 1}>
          {isDesktop && <Box width={PRESENTATION.brandInset / 8} flexShrink={0} />}
          {mark}
          {isDesktop && <Box width={PRESENTATION.brandGap / 8} flexShrink={0} />}
          <Text key="usage-status">Usage unavailable · Last known</Text>
        </Box>
        <Box marginLeft={3}>{meters}</Box>
      </Box>
    )
  }
  return (
    <Box key="usage-band" position="relative" flexDirection="row" alignItems={isDesktop ? 'center' : 'flex-end'} gap={isDesktop ? 0 : 1}>
      {isDesktop && <Box width={PRESENTATION.brandInset / 8} flexShrink={0} />}
      {mark}
      {isDesktop && <Box width={PRESENTATION.brandGap / 8} flexShrink={0} />}
      {meters}
    </Box>
  )
}
export const register: Register = (on) => {
  on('session.start', async ($, e, next) => {
    commandName = undefined
    needsUsageReconciliation = false
    try {
      await resetUsage($)
      await readUsage($)
      const registered = await $.command.register({
        name: 'antiburn-mod',
        description: 'Show account usage, reset times and even pace',
        argumentHint: '[refresh]',
      })
      commandName = registered.command
    } catch {
      // Keep the host session usable when quota setup fails.
    }
    return next(e)
  })

  on('classic.SessionStart', async ($, e, next) => {
    if (e.source === 'clear') {
      try {
        await resetUsage($)
        needsUsageReconciliation = true
        await reconcileUsage($)
      } catch {
        // Keep the host clear operation available when quota refresh fails.
      }
    }
    return next(e)
  })

  on('session.end', async ($, e, next) => {
    try {
      await resetUsage($)
    } catch {
      // End the host session even if display cleanup fails.
    }
    needsUsageReconciliation = e.reason === 'clear' || e.reason === 'resume'
    return next(e)
  })

  on('session.attach', async ($, e, next) => {
    const result = await next(e)
    if (e.surface === 'desktop') await readUsage($)
    return result
  })

  on('session.detach', async ($, e, next) => {
    usageOrder.reset()
    hoverSites.clear()
    try {
      await update($, usage, (current) => ({ ...current, hovered: null }))
    } catch {
      // Detach the client even if hover cleanup fails.
    }
    return next(e)
  })

  on('command.run', async ($, e, next) => {
    if (e.command === 'clear' || e.command === 'resume') {
      const result = await next(e)
      try {
        await reconcileUsage($)
      } catch {
        // Return the host command result when quota refresh fails.
      }
      return result
    }
    if (commandName === undefined || e.command !== commandName) return next(e)
    const argument = e.args.trim()
    if (argument !== '' && argument !== 'details' && argument !== 'refresh') return { text: `Usage: /${commandName} [refresh]` }
    if (argument === 'refresh') await readUsage($)
    const state = await read($, usage)
    const now = await $.clock.now()
    const prefix = state.snapshot.status === 'error' ? 'Usage unavailable · Last known.\n' : ''
    return { text: prefix + PRIMARY_WINDOWS.map((kind) => meterDescription(primaryWindow(state.snapshot, kind), now)).join('\n') }
  })

  on('session.measure', async ($, e, next) => {
    const ticket = usageOrder.acceptEvent()
    needsUsageReconciliation = false
    try {
      await commitUsage($, normalizeUsage(e.rateLimits, await $.clock.now()), ticket)
    } catch {
      try {
        await update($, usage, (current) =>
          usageOrder.isCurrent(ticket) ? { ...current, snapshot: usageFailure(current.snapshot, current.snapshot.capturedAt) } : current,
        )
      } catch {
        // Forward the measurement even if its unavailable label cannot be saved.
      }
    }
    return next(e)
  })

  on('ui.message', async ($, e, next) => {
    const match = /^usage-hover-(reset|pace)-(five_hour|seven_day)$/.exec(e.element)
    if (e.surface !== 'terminal' || e.component !== 'AbovePrompt' || e.module !== 'hooks/usage-target.tsx' || !match) return next(e)
    const data = e.data as { hovered?: unknown; activate?: unknown; binding?: unknown } | null
    if (
      !data ||
      typeof data.hovered !== 'boolean' ||
      typeof data.activate !== 'boolean' ||
      typeof data.binding !== 'string' ||
      data.binding !== hoverSites.get(e.requestId)?.binding
    )
      return next(e)
    const target = match[1] === 'pace' ? `pace-${match[2]}` : match[2]!
    try {
      await update($, usage, (current) => {
        const hovered = data.hovered
          ? { target, requestId: e.requestId, binding: data.binding as string }
          : current.hovered?.target === target && current.hovered.requestId === e.requestId
            ? null
            : current.hovered
        if (!data.activate) return { ...current, hovered }
        const dismissed = current.activated === target
        return { ...current, hovered, revealed: dismissed ? null : target, activated: dismissed ? null : target }
      })
    } catch {
      // Keep host input available when display state cannot be saved.
    }
    return next(e)
  })

  on('ui.close', async ($, e, next) => {
    hoverSites.delete(e.id)
    try {
      await update($, usage, (current) => (current.hovered?.requestId === e.id ? { ...current, hovered: null } : current))
    } catch {
      // Close the host UI even if hover cleanup fails.
    }
    return next(e)
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    if (e.props.hasSurvey || e.props.maxRows <= 0) {
      terminalHoverBinding(e.requestId, 'hidden')
      return next(e)
    }
    try {
      return await renderUsage($, e)
    } catch {
      // Use the host drawing when quota rendering fails.
    }
    return next(e)
  })
}
