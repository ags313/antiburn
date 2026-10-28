import {
  getAgentIntegrations,
  onAgentIntegrationsChanged,
  repairAgentIntegration,
  removeAgentIntegration,
  type IntegrationStatus,
  type IntegrationVendor,
} from "./agentIntegrationsIpc"

type Snapshot = { statuses: IntegrationStatus[]; busy: boolean; error: string | null }

export class AgentIntegrationsSession {
  private listeners = new Set<() => void>()
  private snapshot: Snapshot = { statuses: [], busy: false, error: null }
  private started = false
  private generation = 0
  private stopListening: (() => void) | null = null
  private refreshPending = false

  getSnapshot = (): Snapshot => this.snapshot

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    if (!this.started) {
      this.started = true
      const generation = ++this.generation
      void this.start(generation)
    }
    return () => {
      this.listeners.delete(listener)
      if (this.listeners.size === 0) {
        this.started = false
        this.generation += 1
        this.stopListening?.()
        this.stopListening = null
      }
    }
  }

  reportNavigationError = (): void =>
    this.publish({
      ...this.snapshot,
      error: "Could not open CLI settings. Try again from Settings.",
    })

  repair = (vendor: IntegrationVendor): Promise<void> =>
    this.run(() => repairAgentIntegration(vendor))
  remove = (vendor: IntegrationVendor): Promise<void> =>
    this.run(() => removeAgentIntegration(vendor))
  refresh = async (): Promise<void> => {
    if (this.snapshot.busy) {
      this.refreshPending = true
      return
    }
    await this.run(getAgentIntegrations)
  }

  private async start(generation: number): Promise<void> {
    try {
      const stop = await onAgentIntegrationsChanged(() => {
        if (generation === this.generation) void this.refresh()
      })
      if (generation !== this.generation) {
        stop()
        return
      }
      this.stopListening = stop
    } catch {
      // The status request remains available when event registration fails.
    }
    if (generation === this.generation) await this.refresh()
  }

  private async run(action: () => Promise<IntegrationStatus[]>): Promise<void> {
    if (this.snapshot.busy) return
    const generation = this.generation
    this.publish({ ...this.snapshot, busy: true, error: null })
    let next: Snapshot
    try {
      const statuses = await action()
      next = { statuses, busy: false, error: null }
    } catch {
      next = {
        ...this.snapshot,
        busy: false,
        error: "Could not update agent integrations. Try again.",
      }
    }
    this.publish(generation === this.generation ? next : { ...this.snapshot, busy: false })
    if (this.refreshPending) {
      this.refreshPending = false
      if (this.started) await this.refresh()
    }
  }

  private publish(snapshot: Snapshot): void {
    this.snapshot = snapshot
    for (const listener of this.listeners) listener()
  }
}

export const agentIntegrationsSession = new AgentIntegrationsSession()
