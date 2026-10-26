import { renderNotice, sessionOnce } from '../../../packages/antiburn-plugin-core/core.mjs'
import { discover } from '../../../packages/antiburn-plugin-core/discovery.mjs'

const key = Symbol.for('antiburn.shell.opencode.sessions.v1')
const shared = /** @type {typeof globalThis & {[key: symbol]: Map<string, boolean>}} */ (
  globalThis
)
const sessions = (shared[key] ??= new Map())

/** @type {import('@opencode-ai/plugin').Plugin} */
export const Antiburn = async ({ client, serverUrl }) => {
  const local = ['localhost', '127.0.0.1', '[::1]'].includes(serverUrl.hostname)
  return {
    event: async ({ event }) => {
      if (!local || event.type !== 'session.created' || event.properties.info.parentID) return
      await sessionOnce(sessions, event.properties.info.id, async () => {
        await client.tui.showToast({
          body: { title: 'antiburn', message: renderNotice(await discover()), variant: 'info' },
        })
      })
    },
  }
}
