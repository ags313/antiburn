import { describe } from "../lib/product.mjs"
import { COMMAND } from "./quota-controller"
export {
  ARGUMENT_HINT,
  registerCommands,
} from "./quota-controller"

export const describeCommands = () => describe("claude-code", `/${COMMAND}`, "native")
