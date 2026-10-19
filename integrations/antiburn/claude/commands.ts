import type { On } from "claude-code"
import {
  describe,
  parseUsageCommand,
  renderUsageHelp,
} from "../../../packages/antiburn-plugin-core/product.mjs"

export const ARGUMENT_HINT = "[refresh | help]"

import {
  startQuota,
  endQuota,
  reconcileQuota,
  requestUsage,
  usageDetails,
} from "../../../plugins/antiburn-live/release/hooks/quota-controller"

const COMMAND = "antiburn"
export const describeCommands = () => describe("claude-code", `/${COMMAND}`, "native")

export function registerCommands(on: On, usageUiEnabled: boolean) {
  let registered = false
  let generation = 0

  on("session.start", async ($, e, next) => {
    registered = false
    const current = ++generation
    await startQuota($)
    const result = await next(e)
    if (current !== generation) return result
    try {
      const command = await $.command.register({
        name: COMMAND,
        description: "Show your usage or refresh it",
        argumentHint: ARGUMENT_HINT,
      })
      if (current !== generation) return result
      registered = command.command === COMMAND
    } catch {
      if (current !== generation) return result
    }
    if (!registered) {
      try {
        await $.ui.log("antiburn: /antiburn could not be registered.", { to: "debug" })
      } catch {
        // Keep quota available when the host cannot log the registration failure.
      }
    }
    return result
  })

  on("session.end", async ($, e, next) => {
    await endQuota($, e.reason)
    if (e.reason !== "clear" && e.reason !== "resume") {
      generation++
      registered = false
    }
    return next(e)
  })

  on("ui.render", { component: "CommandOutput" }, (_$, e, next) => {
    if (e.props.command !== COMMAND) return next(e)
    return next({
      ...e,
      props: { ...e.props, text: e.props.text.replace(/^antiburn: /, "") },
    })
  })

  on("command.run", async ($, e, next) => {
    if (e.command === "clear" || e.command === "resume") {
      const result = await next(e)
      await reconcileQuota($)
      return result
    }
    if (!registered || e.command !== COMMAND) return next(e)
    const command = parseUsageCommand(e.args)
    if (command === "invalid" || command === "help")
      return { text: renderUsageHelp(`/${COMMAND}`, command === "invalid") }
    try {
      if (command === "refresh" || !usageUiEnabled) await requestUsage($)
      const confirmation =
        command === "refresh"
          ? usageUiEnabled
            ? "Usage updated."
            : `Usage updated. Run \`/${COMMAND}\` to see it.`
          : undefined
      return await usageDetails($, COMMAND, confirmation)
    } catch {
      return {
        text: `Usage isn’t available right now. Run \`/${COMMAND} refresh\` to try again.`,
      }
    }
  })
}
