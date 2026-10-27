import { useCallback, useSyncExternalStore } from "react"
import type { PromotionSession } from "../../lib/promotions/PromotionSession"
import { useGlobalKeydown } from "../../lib/useGlobalKeydown"
import { PromotionPanel } from "./PromotionPanel"
import { revealPromotion } from "./promotionReveal"

const modalSelector = "dialog[open], [aria-modal='true']"
const hasModal = () => Boolean(document.querySelector(modalSelector))
const noModal = () => false
function subscribeModal(listener: () => void): () => void {
  const observer = new MutationObserver(listener)
  observer.observe(document.body, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["open", "aria-modal"],
  })
  return () => observer.disconnect()
}

export function PromotionHost({
  session,
  blocked = false,
}: {
  session: PromotionSession
  blocked?: boolean
}) {
  const snapshot = useSyncExternalStore(
    session.subscribe,
    session.getSnapshot,
    session.getSnapshot,
  )
  if (!snapshot.selected || !snapshot.visible || blocked) return null
  return <VisiblePromotion session={session} />
}

function VisiblePromotion({ session }: { session: PromotionSession }) {
  const snapshot = useSyncExternalStore(
    session.subscribe,
    session.getSnapshot,
    session.getSnapshot,
  )
  const modal = useSyncExternalStore(subscribeModal, hasModal, noModal)
  useGlobalKeydown(!modal, (event) => {
    if (event.key === "Escape" && !event.defaultPrevented && !event.isComposing) {
      event.preventDefault()
      session.close()
    }
  })
  const attachPanel = useCallback(
    (node: HTMLElement) => {
      if (snapshot.screen !== "catalog") session.presented()
      const stopReveal = session.takeReveal() ? revealPromotion(node) : () => undefined
      const previous = document.activeElement
      if (snapshot.focusPanel) node.focus({ preventScroll: true })
      return () => {
        stopReveal()
        if (
          node.contains(document.activeElement) &&
          previous instanceof HTMLElement &&
          previous.isConnected
        ) {
          previous.focus({ preventScroll: true })
        }
      }
    },
    [session, snapshot.focusPanel, snapshot.screen],
  )
  if (modal) return null
  return (
    <div className="promotion-host">
      <PromotionPanel
        key={snapshot.selected?.id}
        session={session}
        snapshot={snapshot}
        panelRef={attachPanel}
      />
    </div>
  )
}
