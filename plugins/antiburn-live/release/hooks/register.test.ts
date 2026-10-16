import { expect, test } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'
import type { On, RenderElement } from 'claude-code'
import type { RateLimitReading } from './usage'

const PLUGIN = 'antiburn-mod'
const NOW = Date.parse('2026-01-01T00:00:00Z')
const LIMITS = [
  { kind: 'five_hour', percentUsed: 8, resetsAt: '2026-01-01T02:48:00Z' },
  { kind: 'seven_day', percentUsed: 2.5 },
]
const props = (bodyColumns = 120, maxRows = 4, hasSurvey = false) => ({
  bodyColumns,
  maxRows,
  hasSurvey,
  isWorking: false,
  scroll: { offset: 0, bodyRows: maxRows },
  view: {},
})
const command = (name: string, args = '') => ({
  command: name,
  args,
  origin: { kind: 'composer' as const },
  presentation: { isFullscreen: false, columns: 80 },
})

function fakeEngine(on: On) {
  const fake = {
    commandFailure: false,
    commandForwards: 0,
    clockFailure: false,
    now: NOW,
    rateLimits: LIMITS as RateLimitReading[],
    usageReads: 0,
    attachLimits: undefined as RateLimitReading[] | undefined,
    envReads: 0,
    commandName: PLUGIN,
    registered: [] as string[],
    argumentHints: [] as (string | undefined)[],
    forwards: 0,
    renders: 0,
    forbidden: [] as string[],
    writes: [] as string[],
    mode: undefined as string | undefined,
    readUsage: undefined as (() => Promise<RateLimitReading[]>) | undefined,
  }
  on('env.get', () => {
    fake.envReads++
    return { value: fake.mode }
  })
  on('session.usage', async () => {
    fake.usageReads++
    return { value: { startedAt: 0, context: { window: 200000 }, rateLimits: fake.readUsage ? await fake.readUsage() : fake.rateLimits } }
  })
  on('clock.now', () => {
    if (fake.clockFailure) throw new Error('clock unavailable')
    return { value: fake.now }
  })
  on('session.start', () => {
    fake.forwards++
    return { cwd: '/work' }
  })
  on('classic.SessionStart', () => ({}) as never)
  on('session.end', (_$, e) => ({ sessionId: e.sessionId }))
  on('session.attach', (_$, e) => {
    if (fake.attachLimits) fake.rateLimits = fake.attachLimits
    return { clientId: e.clientId }
  })
  on('session.detach', (_$, e) => ({ clientId: e.clientId }))
  on('session.measure', (_$, e) => ({ changed: e.changed }))
  on('state.set', (_$, e, next) => {
    fake.writes.push(e.plugin)
    return next(e)
  })
  on('command.register', (_$, e) => {
    fake.registered.push(e.name)
    fake.argumentHints.push(e.argumentHint)
    return { value: { command: fake.commandName } }
  })
  on('command.run', (_$, e) => {
    fake.commandForwards++
    if (fake.commandFailure) throw new Error('host failure')
    return { text: `host:${e.command}` }
  })
  on('ui.message', () => ({}))
  on('ui.close', () => ({ value: undefined }))
  on('ui.render', ($, e) => {
    fake.renders++
    const { Text } = $.ui.resolve(e)
    return h(Text, {}, 'host drawing') as RenderElement
  })
  on('ui.open', (_$, e) => {
    fake.forbidden.push(`pane:${e.id}`)
    return { value: { isPlaced: true } }
  })
  on('prompt.fill', (_$, e) => {
    fake.forbidden.push('fill')
    return { isFilled: true }
  })
  on('prompt.submit', (_$, e) => {
    fake.forbidden.push('submit')
    return { text: e.text }
  })
  on('session.compact', () => {
    fake.forbidden.push('compact')
    return { messages: [] }
  })
  return fake
}

const start = ($: Engine) => $.session.start({ cwd: '/work', surface: 'terminal', isInteractive: true })
const measure = ($: Engine, rateLimits: RateLimitReading[]) =>
  $.session.measure({ context: { window: 200000 }, rateLimits, changed: ['rateLimits'] })

for (const mode of [undefined, '', 'usage', 'checks', 'CHECKS', 'debug', 'invalid']) {
  test(`release ignores development environment ${String(mode)} and rejects check/debug commands`, async ($, on) => {
    const fake = fakeEngine(on)
    fake.mode = mode
    await start($)
    for (const arg of ['checks', 'debug', 'demo', 'demo tour', 'demo off']) {
      expect((await $.command.run(command(PLUGIN, arg))).text).toBe(`Usage: /${PLUGIN} [refresh]`)
    }
    expect(fake.envReads).toBe(0)
    expect(fake.forbidden).toEqual([])
    expect(fake.registered).toEqual([PLUGIN])
    expect(fake.argumentHints).toEqual(['[refresh]'])
    expect(fake.writes.every((owner) => owner === PLUGIN)).toBe(true)
    expect((await $.command.run(command('antiburn'))).text).toBe('host:antiburn')
  })
}

for (const surface of ['terminal', 'desktop'] as const) {
  test(`native readings retain exact values and unknown windows on ${surface}`, async ($, on) => {
    const fake = fakeEngine(on)
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface, component: 'AbovePrompt', props: props() })
    expect((await band.drawn()) !== null).toBe(true)
    const details = (await $.command.run(command(PLUGIN))).text
    expect(details).toContain('5-hour: 8%. RESETS IN 2H 48M')
    expect(details).toContain('Weekly: 2.5%. Reset unavailable')
    await measure($, [{ kind: 'five_hour', percentUsed: 105 }])
    expect((await $.command.run(command(PLUGIN, 'details'))).text).toContain('5-hour: 105%.')
    expect((await $.command.run(command(PLUGIN))).text).toContain('Weekly: Unavailable.')
    if (surface === 'terminal') {
      expect(await band.find({ text: '105%' })).toBeDefined()
      expect(await band.find({ text: '—' })).toBeDefined()
    } else {
      expect((await band.findAll({ type: 'Svg' })).some((node) => String(node.props.alt).startsWith('5-hour: 105%.'))).toBe(true)
    }
    expect(fake.usageReads).toBe(1)
  })
}

test('clear and resume refresh through host commands and leave no old display state', async ($, on) => {
  const fake = fakeEngine(on)
  await start($)
  const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: props() })
  await band.press({ key: 'usage-detail-five_hour' })
  expect(await band.find({ text: 'RESETS IN 2H 48M' })).toBeDefined()
  for (const reason of ['clear', 'resume'] as const) {
    fake.rateLimits = [{ kind: 'five_hour', percentUsed: reason === 'clear' ? 9 : 10 }]
    await $.session.end({ reason, sessionId: 's', resume: {} } as never)
    expect((await $.command.run(command(reason))).text).toBe(`host:${reason}`)
    expect(await band.find({ text: reason === 'clear' ? '9%' : '10%' })).toBeDefined()
    expect(await band.find({ text: 'RESETS IN 2H 48M' })).toBeUndefined()
  }
  expect(fake.usageReads).toBe(3)
})

test('failed reconciliation reports unavailable and a later native event restores data', async ($, on) => {
  const fake = fakeEngine(on)
  await start($)
  fake.readUsage = async () => {
    throw new Error('unavailable')
  }
  await $.classic.SessionStart({ source: 'clear', model: 'claude-opus-5' })
  const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: props() })
  expect(await band.find({ text: 'Usage unavailable' })).toBeDefined()
  await measure($, LIMITS)
  expect(await band.find({ text: '8%' })).toBeDefined()
  await measure($, [])
  expect(await band.find({ text: 'Waiting for usage data' })).toBeDefined()
  expect(await band.find({ text: '8%' })).toBeUndefined()
})

for (const failure of [false, true]) {
  test(`a delayed initial ${failure ? 'failure' : 'reading'} cannot supersede native measurement`, async ($, on) => {
    const fake = fakeEngine(on)
    let finish!: (limits: RateLimitReading[]) => void
    let fail!: (error: Error) => void
    let entered!: () => void
    const waiting = new Promise<void>((resolve) => {
      entered = resolve
    })
    fake.readUsage = () => {
      entered()
      return new Promise((resolve, reject) => {
        finish = resolve
        fail = reject
      })
    }
    const starting = start($)
    await waiting
    await measure($, [{ kind: 'five_hour', percentUsed: 94 }])
    if (failure) fail(new Error('late failure'))
    else finish(LIMITS)
    await starting
    expect((await $.command.run(command(PLUGIN))).text).toContain('5-hour: 94%.')
    expect((await $.command.run(command(PLUGIN))).text).not.toContain('Last known')
  })
}

test('session end invalidates a pending read before it can restore old quota', async ($, on) => {
  const fake = fakeEngine(on)
  let finish!: (limits: RateLimitReading[]) => void
  let entered!: () => void
  const waiting = new Promise<void>((resolve) => {
    entered = resolve
  })
  fake.readUsage = () => {
    entered()
    return new Promise((resolve) => {
      finish = resolve
    })
  }
  const starting = start($)
  await waiting
  await $.session.end({ reason: 'clear', sessionId: 's', resume: {} } as never)
  finish(LIMITS)
  await starting
  expect((await $.command.run(command(PLUGIN))).text).toContain('5-hour: Unavailable.')
})

test('command dispatch uses the host-assigned name without taking the core command', async ($, on) => {
  const fake = fakeEngine(on)
  fake.commandName = `${PLUGIN}-2`
  await start($)
  expect((await $.command.run(command(fake.commandName))).text).toContain('5-hour: 8%.')
  expect((await $.command.run(command(PLUGIN))).text).toBe(`host:${PLUGIN}`)
  expect((await $.command.run(command('antiburn', 'debug'))).text).toBe('host:antiburn')
})

test('hidden bands forward once and narrow layouts keep details available through the command', async ($, on) => {
  const fake = fakeEngine(on)
  await start($)
  for (const surface of ['terminal', 'desktop'] as const) {
    for (const options of [props(80, 0), props(80, 4, true)]) {
      const band = await $.ui.mount({ plugin: PLUGIN, surface, component: 'AbovePrompt', props: options })
      expect(await band.find({ text: 'host drawing' })).toBeDefined()
    }
    const narrow = await $.ui.mount({ plugin: PLUGIN, surface, component: 'AbovePrompt', props: props(12) })
    expect(await narrow.find({ text: '8%' })).toBeDefined()
  }
  expect(fake.renders >= 4).toBe(true)
  expect((await $.command.run(command(PLUGIN))).text).toContain('Even pace: 44% used by now')
})

test('terminal hover and activation are scoped to the current binding and disappear after clear', async ($, on) => {
  fakeEngine(on)
  await start($)
  const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: props() })
  const key = 'usage-hover-reset-five_hour'
  const old = (await band.find({ key }))!.props.props as { binding: string }
  await band.pointer({ in: key, type: 'move', x: 0, y: 0 })
  expect(await band.find({ text: 'RESETS IN 2H 48M' })).toBeDefined()
  await $.classic.SessionStart({ source: 'clear', model: 'claude-opus-5' })
  await band.post({ binding: old.binding, hovered: true, activate: true }, { in: key })
  expect(await band.find({ text: 'RESETS IN 2H 48M' })).toBeUndefined()
})

test('measurement errors retain labeled last-known values until the next valid event', async ($, on) => {
  const fake = fakeEngine(on)
  await start($)
  fake.clockFailure = true
  await measure($, [{ kind: 'five_hour', percentUsed: 99 }])
  fake.clockFailure = false
  const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: props() })
  expect(await band.find({ text: 'Usage unavailable · Last known' })).toBeDefined()
  expect(await band.find({ text: '8%' })).toBeDefined()
  expect((await $.command.run(command(PLUGIN))).text).toContain('Usage unavailable · Last known.')
  await measure($, [{ kind: 'five_hour', percentUsed: 99 }])
  expect(await band.find({ text: '99%' })).toBeDefined()
  expect(await band.find({ text: 'Usage unavailable · Last known' })).toBeUndefined()
})

test('detach discards hover ownership and preserves the host result', async ($, on) => {
  fakeEngine(on)
  await start($)
  const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: props() })
  const key = 'usage-hover-reset-five_hour'
  await band.pointer({ in: key, type: 'move', x: 0, y: 0 })
  expect(await band.find({ text: 'RESETS IN 2H 48M' })).toBeDefined()
  expect(await $.session.detach({ surface: 'terminal', clientId: 'test-client', reason: 'detach' })).toEqual({ clientId: 'test-client' })
  expect(await band.find({ text: 'RESETS IN 2H 48M' })).toBeUndefined()
})

test('a downstream command failure is forwarded once', async ($, on) => {
  const fake = fakeEngine(on)
  fake.commandFailure = true
  await start($)
  let failed = false
  try {
    await $.command.run(command('resume'))
  } catch {
    failed = true
  }
  expect(failed).toBe(true)
  expect(fake.commandForwards).toBe(1)
})

for (const surface of ['terminal', 'desktop'] as const) {
  test(`release retains the resting brand and quota layout on ${surface}`, async ($, on) => {
    const fake = fakeEngine(on)
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface, component: 'AbovePrompt', props: props() })
    if (surface === 'terminal') {
      const brand = await band.find({ key: 'usage-brand-text-slot' })
      expect(brand?.props.width).toBe(11)
      expect(await band.find({ text: ' antiburn ' })).toBeDefined()
    } else {
      const brand = (await band.findAll({ type: 'Svg' })).find((node) => node.props.alt === 'antiburn')
      expect(brand?.props.width).toBe(14)
      expect(brand?.props.height).toBe(14)
      expect(String(brand?.props.source)).toContain('viewBox="6.2 5.9 32.4 32.4"')
      const meters = await band.findAll({ type: 'Svg' })
      expect(meters.filter((node) => String(node.props.alt).includes('8%.')).length > 0).toBe(true)
      expect((await band.find({ key: 'usage-band' }))?.props.flexDirection).toBe('row')
    }
    await measure($, [])
    if (surface === 'terminal') expect(await band.find({ key: 'usage-brand-text-slot' })).toBeDefined()
    else expect((await band.findAll({ type: 'Svg' })).find((node) => node.props.alt === 'antiburn')).toBeDefined()
    expect(fake.forbidden).toEqual([])
  })
}

test('working and stacked terminal layouts retain both quota readings and reset activation', async ($, on) => {
  fakeEngine(on)
  await start($)
  const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: { ...props(60), isWorking: true } })
  expect((await band.find({ key: 'usage-meters' }))?.props.flexDirection).toBe('column')
  expect(await band.find({ text: '8%' })).toBeDefined()
  expect(await band.find({ text: '2.5%' })).toBeDefined()
  await band.press({ key: 'usage-detail-five_hour' })
  expect(await band.find({ text: 'RESETS IN 2H 48M' })).toBeDefined()
  await band.press({ key: 'usage-detail-five_hour' })
  expect(await band.find({ text: 'RESETS IN 2H 48M' })).toBeUndefined()
})

for (const columns of [24, 40, 80, 120]) {
  test(`Desktop branding and geometry survive empty and error states at ${columns} columns`, async ($, on) => {
    const fake = fakeEngine(on)
    fake.rateLimits = []
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface: 'desktop', component: 'AbovePrompt', props: props(columns) })
    const initial = await band.find({ key: 'usage-band' })
    expect(initial?.props.height).toBe(columns < 80 ? 46 / 16 : 18 / 16)
    const brand = (await band.findAll({ type: 'Svg' })).find((node) => node.props.alt === 'antiburn')
    expect(brand?.props.width).toBe(14)
    fake.readUsage = async () => {
      throw new Error('unavailable')
    }
    await $.classic.SessionStart({ source: 'clear', model: 'claude-opus-5' })
    expect((await band.find({ key: 'usage-band' }))?.props.height).toBe(initial?.props.height)
    expect((await band.findAll({ type: 'Svg' })).some((node) => String(node.props.alt).includes('Usage unavailable'))).toBe(true)
  })
}

for (const surface of ['desktop', 'terminal'] as const) {
  test(`weekly-only limits and numeric boundaries survive the split on ${surface}`, async ($, on) => {
    fakeEngine(on)
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface, component: 'AbovePrompt', props: props() })
    for (const percentUsed of [0, 1, 2.5, 8, 94, 99.5, 100, 105]) {
      await measure($, [{ kind: 'seven_day', percentUsed }])
      const details = (await $.command.run(command(PLUGIN))).text
      expect(details).toContain(`Weekly: ${percentUsed}%.`)
      expect(details).toContain('5-hour: Unavailable. Starts when a message is sent')
      if (surface === 'terminal') expect(await band.find({ text: `${percentUsed}%` })).toBeDefined()
      else
        expect((await band.findAll({ type: 'Svg' })).some((node) => String(node.props.alt).startsWith(`Weekly: ${percentUsed}%.`))).toBe(
          true,
        )
    }
  })
}

test('terminal pace and strip disappearance invalidate stale hover regions', async ($, on) => {
  const fake = fakeEngine(on)
  await start($)
  const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: props() })
  const key = 'usage-hover-pace-five_hour'
  const old = (await band.find({ key }))!.props.props as { binding: string }
  await band.pointer({ in: key, type: 'move', x: 0, y: 0 })
  expect(await band.find({ text: /^Even pace:/ })).toBeDefined()
  fake.now += 3 * 3600000
  await band.redraw()
  expect(await band.find({ key })).toBeUndefined()
  fake.now = NOW
  await band.redraw()
  expect(await band.find({ text: /^Even pace:/ })).toBeUndefined()
  await band.post({ binding: old.binding, hovered: true, activate: false }, { in: key })
  expect(await band.find({ text: /^Even pace:/ })).toBeUndefined()
  await band.pointer({ in: 'usage-hover-reset-five_hour', type: 'move', x: 0, y: 0 })
  await band.redraw(props(120, 4, true))
  await band.redraw(props())
  expect(await band.find({ text: /^RESETS IN / })).toBeUndefined()
})

for (const surface of ['terminal', 'desktop'] as const) {
  test(`secondary-only readings retain unavailable primary meters on ${surface}`, async ($, on) => {
    const fake = fakeEngine(on)
    fake.rateLimits = [{ kind: 'seven_day_opus', percentUsed: 8 }]
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface, component: 'AbovePrompt', props: props() })
    expect(await band.find({ text: 'Waiting for usage data' })).toBeUndefined()
    expect(await band.find({ text: 'Try send a message first' })).toBeUndefined()
    if (surface === 'terminal') expect((await band.findAll({ type: 'Text' })).filter((node) => node.text === '—').length).toBe(2)
    else expect((await band.findAll({ type: 'Svg' })).filter((node) => String(node.props.alt).includes('Unavailable.')).length).toBe(2)
    expect((await $.command.run(command(PLUGIN))).text).toContain('5-hour: Unavailable.')
  })
}

for (const surface of ['desktop', 'terminal'] as const) {
  test(`website activation survives empty, ready and stale states on ${surface}`, async ($, on) => {
    const fake = fakeEngine(on)
    const launches: string[][] = []
    on('process.run', (_$, e) => {
      launches.push([...e.argv])
      return { value: { exitCode: 0, stdout: '', stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
    })
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface, component: 'AbovePrompt', props: props() })
    for (const readings of [[], LIMITS]) {
      await measure($, readings)
      expect(launches).toEqual([])
      if (surface === 'desktop') {
        const links = (await band.findAll({ type: 'Link' })).filter((node) => node.props.href === 'https://antiburn.com')
        expect(links).toHaveLength(1)
        expect(links[0]!.props.label).toBe('Open antiburn website')
      } else {
        expect(await band.find({ key: 'usage-brand-open' })).toBeDefined()
      }
    }
    if (surface === 'terminal') {
      await band.press({ key: 'usage-brand-open' })
      expect(launches).toEqual([['open', 'https://antiburn.com']])
      expect(await band.find({ text: /^RESETS IN / })).toBeUndefined()
    }
    fake.clockFailure = true
    await measure($, LIMITS)
    fake.clockFailure = false
    if (surface === 'desktop')
      expect((await band.findAll({ type: 'Link' })).some((node) => node.props.href === 'https://antiburn.com')).toBe(true)
    else expect(await band.find({ key: 'usage-brand-open' })).toBeDefined()
    expect(fake.forbidden).toEqual([])
  })
}

for (const columns of [60, 120]) {
  test(`terminal reset and pace hover preserve bounds and activated detail at ${columns} columns`, async ($, on) => {
    const fake = fakeEngine(on)
    fake.rateLimits = [
      { kind: 'five_hour', percentUsed: 48, resetsAt: '2026-01-01T03:00:00Z' },
      { kind: 'seven_day', percentUsed: 82, resetsAt: '2026-01-04T00:00:00Z' },
    ]
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: props(columns) })
    const reset = 'usage-hover-reset-seven_day'
    const marker = 'usage-hover-pace-seven_day'
    expect((await band.find({ key: reset }))!.props.width).toBe(6)
    expect((await band.find({ key: marker }))!.props.width).toBe(1)
    await band.pointer({ in: reset, type: 'move', x: 2, y: 0 })
    expect(await band.find({ text: /^RESETS IN 3D/ })).toBeDefined()
    await band.pointer({ in: reset, type: 'leave', x: 2, y: 0 })
    expect(await band.find({ text: /^RESETS IN / })).toBeUndefined()
    await band.press({ key: 'usage-detail-seven_day' })
    await band.pointer({ in: marker, type: 'move', x: 0, y: 0 })
    expect(await band.find({ text: /^Even pace:/ })).toBeDefined()
    expect(await band.find({ text: /^RESETS IN / })).toBeUndefined()
    await band.pointer({ in: marker, type: 'move', x: 1, y: 0 })
    expect(await band.find({ text: /^Even pace:/ })).toBeUndefined()
    expect(await band.find({ text: /^RESETS IN 3D/ })).toBeDefined()
  })
}

for (const scenario of ['windows', 'linux-fallback', 'unavailable'] as const) {
  test(`terminal website opener stays bounded for ${scenario}`, async ($, on) => {
    const fake = fakeEngine(on)
    fake.mode = scenario === 'windows' ? 'Windows_NT' : undefined
    const launches: { argv: string[]; timeoutMs?: number }[] = []
    const statuses: (string | undefined)[] = []
    on('ui.status', (_$, e) => {
      statuses.push(e.text)
      return { value: undefined }
    })
    on('process.run', (_$, e) => {
      launches.push({ argv: [...e.argv], timeoutMs: e.init?.timeoutMs })
      if (scenario === 'unavailable') throw new Error('opener not installed')
      return {
        value: {
          exitCode: scenario === 'linux-fallback' && e.argv[0] === 'open' ? 1 : 0,
          stdout: '',
          stderr: '',
          isStdoutTruncated: false,
          isStderrTruncated: false,
        },
      }
    })
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface: 'terminal', component: 'AbovePrompt', props: props() })
    expect(launches).toEqual([])
    await band.press({ key: 'usage-brand-open' })
    expect(launches).toEqual(
      scenario === 'windows'
        ? [{ argv: ['cmd.exe', '/c', 'start', '', 'https://antiburn.com'], timeoutMs: 5000 }]
        : [
            { argv: ['open', 'https://antiburn.com'], timeoutMs: 5000 },
            { argv: ['xdg-open', 'https://antiburn.com'], timeoutMs: 5000 },
          ],
    )
    expect(statuses).toEqual(scenario === 'unavailable' ? ['Open https://antiburn.com in your browser'] : [])
  })
}

for (const surface of ['desktop', 'terminal'] as const) {
  test(`refresh recovers late host readings without a model request on ${surface}`, async ($, on) => {
    const fake = fakeEngine(on)
    fake.rateLimits = []
    await start($)
    const band = await $.ui.mount({ plugin: PLUGIN, surface, component: 'AbovePrompt', props: props() })
    expect(await band.find({ key: 'usage-refresh' })).toBeDefined()
    fake.rateLimits = LIMITS
    await band.press({ key: 'usage-refresh' })
    expect((await $.command.run(command(PLUGIN))).text).toContain('5-hour: 8%.')
    expect(fake.usageReads).toBe(2)
    expect(await band.find({ key: 'usage-refresh' })).toBeUndefined()
    expect(fake.forbidden).toEqual([])
  })
}

test('Desktop attachment re-reads after the host attaches without sending a prompt', async ($, on) => {
  const fake = fakeEngine(on)
  fake.rateLimits = []
  await start($)
  fake.attachLimits = LIMITS
  await $.session.attach({ surface: 'desktop', clientId: 'desktop:test' })
  expect((await $.command.run(command(PLUGIN))).text).toContain('5-hour: 8%.')
  expect(fake.usageReads).toBe(2)
  expect(fake.forbidden).toEqual([])
})

test('manual refresh remains available when the host has no quota yet', async ($, on) => {
  const fake = fakeEngine(on)
  fake.rateLimits = []
  await start($)
  expect((await $.command.run(command(PLUGIN, 'refresh'))).text).toContain('5-hour: Unavailable.')
  expect(fake.usageReads).toBe(2)
  expect(fake.forbidden).toEqual([])
  fake.rateLimits = LIMITS
  expect((await $.command.run(command(PLUGIN, 'refresh'))).text).toContain('5-hour: 8%.')
  expect(fake.usageReads).toBe(3)
})

test('terminal attachment forwards without another usage read', async ($, on) => {
  const fake = fakeEngine(on)
  await start($)
  expect(await $.session.attach({ surface: 'terminal', clientId: 'terminal:test' })).toEqual({ clientId: 'terminal:test' })
  expect(fake.usageReads).toBe(1)
})

test('a pending Desktop refresh cannot overwrite a newer native measurement', async ($, on) => {
  const fake = fakeEngine(on)
  await start($)
  let finish!: (limits: RateLimitReading[]) => void
  let entered!: () => void
  const waiting = new Promise<void>((resolve) => {
    entered = resolve
  })
  fake.readUsage = () => {
    entered()
    return new Promise((resolve) => {
      finish = resolve
    })
  }
  const attaching = $.session.attach({ surface: 'desktop', clientId: 'desktop:late' })
  await waiting
  await measure($, [{ kind: 'five_hour', percentUsed: 94 }])
  finish(LIMITS)
  await attaching
  expect((await $.command.run(command(PLUGIN))).text).toContain('5-hour: 94%.')
  expect(fake.forbidden).toEqual([])
})
