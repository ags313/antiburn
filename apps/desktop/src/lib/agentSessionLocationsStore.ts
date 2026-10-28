/** Session roots are cached until an explicit agent rescan refreshes them. */
import { agentSessionLocations, type AgentSessionLocations } from "./ipc"

let cache: AgentSessionLocations[] | null = null
let inFlight: Promise<void> | null = null
let initialRead = false
let queuedRefresh: Promise<void> | null = null
const listeners = new Set<() => void>()

function loadRoots(initial = false): Promise<void> {
  if (inFlight) return inFlight
  initialRead = initial
  inFlight = agentSessionLocations()
    .then((locations) => {
      cache = locations
    })
    .finally(() => {
      inFlight = null
      initialRead = false
      for (const listener of listeners) listener()
    })
  return inFlight
}

/** Refresh root existence after any pending initial lookup completes. */
export function refreshAgentSessionLocations(): Promise<void> {
  if (queuedRefresh) return queuedRefresh
  if (inFlight && initialRead) {
    queuedRefresh = inFlight
      .catch(() => undefined)
      .then(() => loadRoots())
      .finally(() => {
        queuedRefresh = null
      })
    return queuedRefresh
  }
  return loadRoots()
}

export function subscribeAgentSessionLocations(listener: () => void): () => void {
  listeners.add(listener)
  if (cache === null)
    void loadRoots(true).catch(() => {
      cache ??= []
      for (const notify of listeners) notify()
    })
  return () => listeners.delete(listener)
}

/** Null means the initial root read has not completed. */
export function agentSessionLocationsSnapshot(): AgentSessionLocations[] | null {
  return cache
}
