import { describe } from "../../../packages/antiburn-plugin-core/product.mjs"
import { COMMAND } from "../../../plugins/antiburn-live/release/hooks/quota-controller"
export {
  ARGUMENT_HINT,
  registerCommands,
} from "../../../plugins/antiburn-live/release/hooks/quota-controller"

export const describeCommands = () => describe("claude-code", `/${COMMAND}`, "native")
