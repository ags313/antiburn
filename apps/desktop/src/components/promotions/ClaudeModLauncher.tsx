import { ArrowRight } from "lucide-react"
import { useState, useSyncExternalStore, type Ref } from "react"
import { renderAgentIcon } from "../../lib/agentIcon"
import { createExternalStore } from "../../lib/externalStore"
import { EMPTY_LIVE_USAGE, getLiveUsage, onLiveUsageChanged } from "../../lib/ipc"
import { agentStatus, meterForAgent } from "../../lib/presentation/agentStatus"
import type { PromotionSession } from "../../lib/promotions/PromotionSession"
import {
  agentSessionCounts,
  subscribeAgentSessionCounts,
} from "../../views/main-window/overview/stepSettings/agentSessionCounts"

export function ClaudeModLauncher({
  session,
  onOpen,
  buttonRef,
}: {
  session: PromotionSession
  onOpen: () => void
  buttonRef?: Ref<HTMLButtonElement>
}) {
  const counts = useSyncExternalStore(subscribeAgentSessionCounts, agentSessionCounts)
  const [liveStore] = useState(() =>
    createExternalStore({
      initial: EMPTY_LIVE_USAGE,
      load: () => getLiveUsage().catch(() => EMPTY_LIVE_USAGE),
      subscribe: onLiveUsageChanged,
    }),
  )
  const live = useSyncExternalStore(liveStore.subscribe, liveStore.getSnapshot)
  useSyncExternalStore(session.subscribe, session.getSnapshot)
  const promo = session.catalog.find(
    (promo) => promo.id === "claude-mod" && !promo.development_only,
  )
  const count = counts.find((entry) => entry.agent === "claude-code")?.sessions ?? 0
  const detected = agentStatus(count, meterForAgent("claude-code", live.meters)).found
  if (!promo || !detected) return null
  return (
    <button
      ref={buttonRef}
      type="button"
      className="promotion-launcher flex w-full flex-col gap-1 rounded-popover p-3 text-left text-label"
      aria-label="Claude Mod, new"
      aria-haspopup="dialog"
      onClick={(event) => {
        if (session.openFromLauncher(promo.id, event.detail > 0)) onOpen()
      }}
    >
      <span className="flex w-full items-center gap-2 type-body">
        <span aria-hidden="true" className="shrink-0">
          {renderAgentIcon("claude-code", 14)}
        </span>
        <span className="whitespace-nowrap font-medium">Claude Mod</span>
        <span
          className="promotion-launcher-badge shrink-0 rounded-control bg-surface-secondary px-1 type-caption font-medium text-label-secondary"
          aria-hidden="true"
        >
          NEW
        </span>
      </span>
      <span className="type-caption text-label-secondary">
        Usage, resets, and pace above your prompt.{" "}
        <span className="inline-flex items-center gap-1 whitespace-nowrap font-medium text-label-secondary">
          Find out more <ArrowRight size={12} aria-hidden="true" />
        </span>
      </span>
    </button>
  )
}
