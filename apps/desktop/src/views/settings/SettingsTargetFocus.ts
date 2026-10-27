/** Reveal each explicit search target once, including controls that load later. */
export class SettingsTargetFocus {
  private applied: string | null = null
  private recoveryKey: string | null = null
  private recoveryRow: HTMLElement | null = null

  attach(
    node: HTMLDivElement,
    pane: string,
    control: string | null | undefined,
    revision: number | undefined,
  ): () => void {
    const key = `${pane}:${control ?? ""}:${revision ?? 0}`
    if (this.applied === key) return () => undefined
    const reveal = () => {
      if (!control) {
        const viewport = node.closest<HTMLDivElement>(".ui-scroll-viewport")
        if (viewport) viewport.scrollTop = 0
        this.applied = key
        return true
      }
      const row = [...node.querySelectorAll<HTMLElement>("[data-settings-control]")].find(
        (element) => element.dataset.settingsControl === control,
      )
      if (!row) return false
      if (row.closest('[aria-busy="true"]') || row.querySelector('[aria-busy="true"]'))
        return false
      const pending = row.hasAttribute("data-settings-target-pending")
      if (pending && this.recoveryKey === key && this.recoveryRow === row) return false
      row.scrollIntoView({ block: "center", behavior: "instant" })
      const field = row.hasAttribute("data-settings-focus-self")
        ? null
        : row.querySelector<HTMLElement>(
            "button:not(:disabled):not([aria-disabled=true]), input:not(:disabled):not([aria-disabled=true]), select:not(:disabled):not([aria-disabled=true]), [role=switch]:not(:disabled):not([aria-disabled=true])",
          )
      ;(field ?? row).focus({ preventScroll: true })
      if (pending) {
        this.recoveryKey = key
        this.recoveryRow = row
        return false
      }
      this.applied = key
      this.recoveryKey = null
      this.recoveryRow = null
      return true
    }
    if (reveal()) return () => undefined
    const observer = new MutationObserver(() => {
      if (reveal()) observer.disconnect()
    })
    observer.observe(node, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: [
        "aria-busy",
        "disabled",
        "aria-disabled",
        "data-settings-target-pending",
      ],
    })
    return () => observer.disconnect()
  }
}
