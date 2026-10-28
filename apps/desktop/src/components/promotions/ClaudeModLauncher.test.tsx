import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { AgentFoundCount } from "../../lib/ipc"
import type { LiveUsageMeterPayload, LiveUsageSummaryPayload } from "../../lib/providerUsageIpc"
import {
  PromotionSession,
  type PromotionEnvironment,
} from "../../lib/promotions/PromotionSession"
import type { PromotionDefinition } from "../../promotions/types"
import { ClaudeModLauncher } from "./ClaudeModLauncher"

const detection = vi.hoisted(() => ({
  counts: [] as AgentFoundCount[],
  listeners: new Set<() => void>(),
  live: { providers: [], meters: [], errors: [], generatedAt: "" } as LiveUsageSummaryPayload,
  changed: (_live: LiveUsageSummaryPayload) => {},
  fail: false,
}))
vi.mock("../../views/main-window/overview/stepSettings/agentSessionCounts", () => ({
  agentSessionCounts: () => detection.counts,
  subscribeAgentSessionCounts: (listener: () => void) => {
    detection.listeners.add(listener)
    return () => detection.listeners.delete(listener)
  },
}))
vi.mock("../../lib/ipc", () => ({
  EMPTY_LIVE_USAGE: { providers: [], meters: [], errors: [], generatedAt: "" },
  getLiveUsage: async () => {
    if (detection.fail) throw new Error("unavailable")
    return detection.live
  },
  onLiveUsageChanged: async (listener: (live: LiveUsageSummaryPayload) => void) => {
    detection.changed = listener
    return () => {
      detection.changed = () => {}
    }
  },
  noteInteraction: vi.fn(),
}))
const promo: PromotionDefinition = {
  schema_version: 1,
  id: "claude-mod",
  revision: 1,
  title: "Claude Mod",
  screens: { announcement: { title: "Claude Mod", description: "Description" } },
}
const claude = (fields: Partial<LiveUsageMeterPayload>): LiveUsageMeterPayload => ({
  provider: "anthropic",
  displayName: "Claude",
  shown: true,
  ...fields,
})
const environment = (): PromotionEnvironment => ({
  isManualReady: () => true,
  loadReady: async () => false,
  onReady: async () => () => {},
  loadVisible: async () => true,
  onVisible: async () => () => {},
  isForeground: () => true,
  onForeground: () => () => {},
  readSeen: () => '["claude-mod@1"]',
  writeSeen: vi.fn(),
  readLauncherOpened: () => null,
  writeLauncherOpened: vi.fn(),
})
async function mount(catalog = [promo]) {
  const session = new PromotionSession(catalog, environment())
  const onOpen = vi.fn()
  render(<ClaudeModLauncher session={session} onOpen={onOpen} />)
  await act(async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve()
  })
  return { session, onOpen }
}
beforeEach(() => {
  detection.counts = []
  detection.live = { providers: [], meters: [], errors: [], generatedAt: "" }
  detection.fail = false
})
afterEach(cleanup)

describe("Claude Mod sidebar launcher", () => {
  it.each([
    ["Claude sessions", 2, undefined, true],
    ["installation", 0, claude({ detection: "installedNotSignedIn" }), true],
    ["sign-in", 0, claude({ detection: "signedIn" }), true],
    [
      "Claude Desktop",
      0,
      claude({ detection: "notInstalled", desktopAppLabel: "Claude Desktop" }),
      true,
    ],
    ["hidden meter", 0, claude({ detection: "signedIn", shown: false }), true],
    ["Pi-only login", 0, claude({ detection: "signedIn", carrierLabel: "Pi" }), false],
    ["unknown detection", 0, undefined, false],
    ["undetected", 0, claude({ detection: "notInstalled" }), false],
  ] as const)(
    "uses shared Agents detection for %s",
    async (_label, sessions, meter, visible) => {
      detection.counts = [{ agent: "claude-code", sessions, done: true }]
      detection.live = { ...detection.live, meters: meter ? [meter] : [] }
      await mount()
      expect(screen.queryByRole("button", { name: "Claude Mod, new" }) !== null).toBe(visible)
    },
  )
  it("keeps unknown and failed detection hidden without treating other agents as Claude", async () => {
    detection.counts = [{ agent: "codex", sessions: 10, done: true }]
    detection.fail = true
    await mount()
    expect(screen.queryByRole("button")).toBeNull()
  })
  it("updates from live detection events without starting a provider refresh", async () => {
    await mount()
    expect(screen.queryByRole("button")).toBeNull()
    act(() =>
      detection.changed({ ...detection.live, meters: [claude({ detection: "signedIn" })] }),
    )
    expect(screen.getByRole("button", { name: "Claude Mod, new" })).toBeVisible()
    act(() => detection.changed(detection.live))
    expect(screen.queryByRole("button")).toBeNull()
  })
  it("opens one named announcement, retains NEW, and keeps the launcher available", async () => {
    detection.counts = [{ agent: "claude-code", sessions: 1, done: true }]
    const { session, onOpen } = await mount()
    fireEvent.click(screen.getByRole("button", { name: "Claude Mod, new" }))
    expect(onOpen).toHaveBeenCalledOnce()
    expect(session.takeReveal()).toBe(false)
    expect(session.getSnapshot()).toMatchObject({
      selected: promo,
      screen: "announcement",
      manual: true,
    })
    expect(screen.getByRole("button", { name: "Claude Mod, new" })).toBeInTheDocument()
    expect(screen.getByText("NEW")).toBeVisible()
    act(() => session.close())
    fireEvent.click(screen.getByRole("button", { name: "Claude Mod, new" }), {
      detail: 1,
    })
    expect(onOpen).toHaveBeenCalledTimes(2)
    expect(session.takeReveal()).toBe(true)
  })
  it("hides the launcher if the shipped campaign is missing or development-only", async () => {
    detection.counts = [{ agent: "claude-code", sessions: 1, done: true }]
    await mount([{ ...promo, development_only: true }])
    expect(screen.queryByRole("button")).toBeNull()
  })
})
