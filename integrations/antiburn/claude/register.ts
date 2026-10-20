import type { Register } from "claude-code"
import { describeCommands, registerCommands } from "./commands"
import { registerQuota } from "../../../plugins/antiburn-live/release/hooks/quota-controller"

export const register: Register = (on) => {
  const usageUiEnabled = true
  registerCommands(on, usageUiEnabled)

  on("engine.create", async ($, e, next) => {
    const engine = await next(e)
    return {
      ...engine,
      antiburn: {
        describe: async () => {
          const description = describeCommands()
          return {
            ...description,
            capabilities: { ...description.capabilities, usage: true, usageUiEnabled },
          }
        },
      },
    }
  })
  registerQuota(on, { usageUiEnabled })
}
