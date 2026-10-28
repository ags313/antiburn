import { useSyncExternalStore, type RefObject } from "react"
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
  returnFocusTo,
}: {
  session: PromotionSession
  blocked?: boolean
  returnFocusTo?: RefObject<HTMLElement | null>
}) {
  const snapshot = useSyncExternalStore(
    session.subscribe,
    session.getSnapshot,
    session.getSnapshot,
  )
  if (!snapshot.selected || !snapshot.visible || blocked) return null
  return <VisiblePromotion session={session} returnFocusTo={returnFocusTo} />
}

function VisiblePromotion({
  session,
  returnFocusTo,
}: {
  session: PromotionSession
  returnFocusTo?: RefObject<HTMLElement | null> | undefined
}) {
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
  const attachPanel = (node: HTMLElement) => {
    if (snapshot.screen !== "catalog") session.presented()
    const stopReveal = session.takeReveal() ? revealPromotion(node) : () => undefined
    const previous = returnFocusTo?.current ?? document.activeElement
    if (snapshot.focusPanel) node.focus({ preventScroll: true })
    return () => {
      stopReveal()
      const ownedFocus = node.contains(document.activeElement)
      queueMicrotask(() => {
        if (
          ownedFocus &&
          !node.isConnected &&
          !hasModal() &&
          previous instanceof HTMLElement &&
          previous.isConnected
        ) {
          previous.focus({ preventScroll: true })
        }
      })
    }
  }
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
