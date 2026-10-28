import { beforeEach, expect, it, vi } from "vitest"
import { AgentIntegrationsSession } from "./agentIntegrationsSession"
import type { IntegrationStatus } from "./agentIntegrationsIpc"

const api = vi.hoisted(() => ({
  getAgentIntegrations: vi.fn(),
  onAgentIntegrationsChanged: vi.fn(),
  repairAgentIntegration: vi.fn(),
  removeAgentIntegration: vi.fn(),
}))
vi.mock("./agentIntegrationsIpc", () => api)

const pending: IntegrationStatus[] = [
  {
    vendor: "codex",
    phase: "pending_native_gate",
    reason: "native_gate_pending",
    hostVersion: "test",
    profile: null,
    nodeVersion: null,
    owned: false,
  },
]

beforeEach(() => {
  vi.resetAllMocks()
  api.getAgentIntegrations.mockResolvedValue(pending)
  api.onAgentIntegrationsChanged.mockResolvedValue(() => {})
})

it("binds events before reading and refreshes an event received during a request", async () => {
  const order: string[] = []
  let changed = () => {}
  let resolveFirst: (value: IntegrationStatus[]) => void = () => {}
  api.onAgentIntegrationsChanged.mockImplementation(async (callback: () => void) => {
    order.push("listen")
    changed = callback
    return () => {}
  })
  api.getAgentIntegrations.mockImplementationOnce(() => {
    order.push("get")
    return new Promise<IntegrationStatus[]>((resolve) => {
      resolveFirst = resolve
    })
  })
  const session = new AgentIntegrationsSession()
  const stop = session.subscribe(() => {})
  await vi.waitFor(() => expect(order).toEqual(["listen", "get"]))
  changed()
  resolveFirst([])
  await vi.waitFor(() => expect(session.getSnapshot().statuses).toEqual(pending))
  expect(api.getAgentIntegrations).toHaveBeenCalledTimes(2)
  expect(api.repairAgentIntegration).not.toHaveBeenCalled()
  expect(api.removeAgentIntegration).not.toHaveBeenCalled()
  stop()
})

it("cleans a listener that finishes registration after unmount", async () => {
  const unlisten = vi.fn()
  let resolveListen: (stop: () => void) => void = () => {}
  api.onAgentIntegrationsChanged.mockImplementation(
    () =>
      new Promise<() => void>((resolve) => {
        resolveListen = resolve
      }),
  )
  const session = new AgentIntegrationsSession()
  const stop = session.subscribe(() => {})
  stop()
  resolveListen(unlisten)
  await vi.waitFor(() => expect(unlisten).toHaveBeenCalledOnce())
  expect(api.getAgentIntegrations).not.toHaveBeenCalled()
})

it("retains status and hides raw errors when an explicit repair fails", async () => {
  const session = new AgentIntegrationsSession()
  const stop = session.subscribe(() => {})
  await vi.waitFor(() => expect(session.getSnapshot().statuses).toEqual(pending))
  api.repairAgentIntegration.mockRejectedValue(new Error("private profile path"))
  await session.repair("codex")
  expect(session.getSnapshot().statuses).toEqual(pending)
  expect(session.getSnapshot().error).toBe("Could not update agent integrations. Try again.")
  stop()
})

it.each(["success", "failure"])(
  "loads fresh state after remount while an old %s is pending",
  async (result) => {
    let resolveOld: (value: IntegrationStatus[]) => void = () => {}
    let rejectOld: (error: Error) => void = () => {}
    let resolveFresh: (value: IntegrationStatus[]) => void = () => {}
    api.getAgentIntegrations
      .mockImplementationOnce(
        () =>
          new Promise<IntegrationStatus[]>((resolve, reject) => {
            resolveOld = resolve
            rejectOld = reject
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<IntegrationStatus[]>((resolve) => {
            resolveFresh = resolve
          }),
      )
    const session = new AgentIntegrationsSession()
    const stop = session.subscribe(() => {})
    await vi.waitFor(() => expect(api.getAgentIntegrations).toHaveBeenCalledTimes(1))
    stop()
    const snapshots: ReturnType<typeof session.getSnapshot>[] = []
    const stopAgain = session.subscribe(() => snapshots.push(session.getSnapshot()))
    await vi.waitFor(() => expect(api.onAgentIntegrationsChanged).toHaveBeenCalledTimes(2))
    if (result === "success") resolveOld([{ ...pending[0]!, reason: "stale" }])
    else rejectOld(new Error("old request failed"))
    await vi.waitFor(() => expect(api.getAgentIntegrations).toHaveBeenCalledTimes(2))
    expect(
      snapshots.some(
        (snapshot) =>
          snapshot.statuses.some((status) => status.reason === "stale") ||
          snapshot.error !== null,
      ),
    ).toBe(false)
    resolveFresh(pending)
    await vi.waitFor(() => expect(session.getSnapshot().statuses).toEqual(pending))
    expect(session.getSnapshot().busy).toBe(false)
    stopAgain()
  },
)
