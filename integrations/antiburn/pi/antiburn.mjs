import {
  describe,
  parseCommand,
  renderHelp,
  renderNotice,
  renderStatus,
  sessionOnce,
} from '../../../packages/antiburn-plugin-core/core.mjs'
import { discover } from '../../../packages/antiburn-plugin-core/discovery.mjs'

const key = Symbol.for('antiburn.shell.sessions.v1')
const shared = /** @type {typeof globalThis & {[key: symbol]: Map<string, boolean>}} */ (
  globalThis
)
const sessions = (shared[key] ??= new Map())

/** @param {import('@earendil-works/pi-coding-agent').ExtensionAPI} pi */
export default function antiburn(pi) {
  pi.registerCommand('antiburn', {
    description: 'antiburn help and executable availability',
    handler: async (args, ctx) => {
      if (!ctx.hasUI) return
      const parsed = parseCommand(args)
      const meta = describe('pi', '/antiburn', 'native', ctx.mode)
      const sessionId = ctx.sessionManager.getSessionId()
      if (parsed.command === 'status') {
        await sessionOnce(sessions, sessionId, async () => {})
        ctx.ui.notify(renderStatus(meta, await discover(), parsed.style), 'info')
        return
      }
      let firstNotice = ''
      if (parsed.command === 'help') {
        await sessionOnce(sessions, sessionId, async () => {
          firstNotice = renderNotice({ state: 'unavailable' }, parsed.style)
          firstNotice = renderNotice(await discover(), parsed.style)
        })
      }
      const help = renderHelp(meta, parsed.style, parsed.command === 'usage')
      const text = firstNotice ? [firstNotice, ...help.split('\n').slice(1)].join('\n') : help
      ctx.ui.notify(text, 'info')
    },
  })
  pi.on('session_start', async (_event, ctx) => {
    if (ctx.mode !== 'tui' || !ctx.hasUI) return
    await sessionOnce(sessions, ctx.sessionManager.getSessionId(), async () => {
      ctx.ui.notify(renderNotice(await discover()), 'info')
    })
  })
}
