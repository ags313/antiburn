import { beforeEach, expect, it, vi } from "vitest"
const load = vi.hoisted(() => vi.fn())
vi.mock("./ipc", () => ({ agentSessionLocations: load }))
beforeEach(() => {
  vi.resetModules()
  load.mockReset()
})
it("refreshes root existence and publishes the new snapshot", async () => {
  const store = await import("./agentSessionLocationsStore")
  const notify = vi.fn()
  load.mockResolvedValueOnce([
    { agent: "codex", locations: [{ path: "~/.codex", found: false }] },
  ])
  await store.refreshAgentSessionLocations()
  const stop = store.subscribeAgentSessionLocations(notify)
  expect(store.agentSessionLocationsSnapshot()?.[0]?.locations[0]?.found).toBe(false)
  load.mockResolvedValueOnce([
    { agent: "codex", locations: [{ path: "~/.codex", found: true }] },
  ])
  await store.refreshAgentSessionLocations()
  expect(store.agentSessionLocationsSnapshot()?.[0]?.locations[0]?.found).toBe(true)
  expect(notify).toHaveBeenCalledTimes(1)
  stop()
})
it("coalesces concurrent refreshes and preserves prior data after a failure", async () => {
  const store = await import("./agentSessionLocationsStore")
  load.mockResolvedValueOnce([{ agent: "codex", locations: [] }])
  const first = store.refreshAgentSessionLocations()
  expect(store.refreshAgentSessionLocations()).toBe(first)
  await first
  const snapshot = store.agentSessionLocationsSnapshot()
  load.mockRejectedValueOnce(new Error("read failed"))
  await expect(store.refreshAgentSessionLocations()).rejects.toThrow("read failed")
  expect(store.agentSessionLocationsSnapshot()).toBe(snapshot)
})
it("performs a fresh read when rescan overlaps the initial lookup", async () => {
  const store = await import("./agentSessionLocationsStore")
  let finish!: (value: unknown) => void
  load.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve
      }),
  )
  load.mockResolvedValueOnce([
    { agent: "codex", locations: [{ path: "~/.codex", found: true }] },
  ])
  store.subscribeAgentSessionLocations(vi.fn())
  const rescan = store.refreshAgentSessionLocations()
  expect(store.refreshAgentSessionLocations()).toBe(rescan)
  expect(load).toHaveBeenCalledTimes(1)
  finish([{ agent: "codex", locations: [{ path: "~/.codex", found: false }] }])
  await rescan
  expect(load).toHaveBeenCalledTimes(2)
  expect(store.agentSessionLocationsSnapshot()?.[0]?.locations[0]?.found).toBe(true)
})
it("retries after an overlapping initial lookup fails", async () => {
  const store = await import("./agentSessionLocationsStore")
  load.mockRejectedValueOnce(new Error("read failed"))
  load.mockResolvedValueOnce([{ agent: "codex", locations: [] }])
  store.subscribeAgentSessionLocations(vi.fn())
  await store.refreshAgentSessionLocations()
  expect(store.agentSessionLocationsSnapshot()).toHaveLength(1)
})
