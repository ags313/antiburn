import type { ReactNode } from "react"

import { ScrollPane } from "../../../components/ui/ScrollPane"
import { BURN_CHECK_MARKS } from "../../../components/burn-checks/burnCheckMarks"

export function PassedCheckDetail({ children }: { children: ReactNode }) {
  const PassIcon = BURN_CHECK_MARKS.clean.Icon
  return (
    <div className="flex items-start gap-3 rounded-control bg-surface-card/75 p-4">
      <PassIcon
        size={14}
        strokeWidth={BURN_CHECK_MARKS.clean.strokeWidth}
        className={`mt-0.5 ${BURN_CHECK_MARKS.clean.iconClass}`}
        aria-hidden="true"
      />
      <p className="type-callout text-label-secondary">{children}</p>
    </div>
  )
}

function observeSpace(node: HTMLDivElement | null) {
  if (!node || typeof ResizeObserver === "undefined") return
  const content = node.querySelector<HTMLElement>(".burn-checks-detail-content")
  const mark = node.querySelector<HTMLElement>(".burn-check-background-mark")
  if (!content || !mark) return
  const update = () => {
    node.dataset.watermarkRoom = String(
      node.clientHeight - content.offsetHeight >= mark.offsetHeight,
    )
  }
  const observer = new ResizeObserver(update)
  observer.observe(node)
  observer.observe(content)
  update()
  return () => {
    observer.disconnect()
    delete node.dataset.watermarkRoom
  }
}

export function BurnCheckDetailBody({
  visible,
  children,
}: {
  visible: boolean
  children: ReactNode
}) {
  return (
    <div className="burn-check-detail-body" ref={visible ? observeSpace : undefined}>
      <div className="burn-check-background-mark" aria-hidden="true" />
      <ScrollPane
        className="relative z-10 min-h-0"
        topEdgeFade
        viewportClassName="burn-checks-detail-scroll"
      >
        {children}
      </ScrollPane>
    </div>
  )
}
