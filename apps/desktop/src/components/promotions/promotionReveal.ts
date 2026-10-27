/** Bridge the first appearance without delaying interaction. */
export function revealPromotion(panel: HTMLElement): () => void {
  if (typeof panel.animate !== "function") return () => undefined
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)")
  const style = getComputedStyle(panel)
  const duration = Number.parseFloat(
    style.getPropertyValue(reduced.matches ? "--duration-quick" : "--duration-medium"),
  )
  const animation = panel.animate(
    reduced.matches
      ? [{ opacity: 0 }, { opacity: 1 }]
      : [
          {
            opacity: 0,
            transform: `translateY(${style.getPropertyValue("--space-sm").trim()})`,
          },
          { opacity: 1, transform: "translateY(0)" },
        ],
    { duration, easing: style.getPropertyValue("--ease-out-quart").trim() },
  )
  const finish = () => animation.finish()
  const stopListening = () => reduced.removeEventListener("change", finish)
  reduced.addEventListener("change", finish)
  animation.addEventListener("finish", stopListening, { once: true })
  animation.addEventListener("cancel", stopListening, { once: true })
  return () => {
    queueMicrotask(() => {
      if (!panel.isConnected) animation.cancel()
    })
  }
}
