import { afterEach, describe, expect, it, vi } from "vitest"
import { SettingsTargetFocus } from "./SettingsTargetFocus"
afterEach(() => document.body.replaceChildren())
describe("settings search focus", () => {
  it("focuses an explicit group target without choosing one of its controls", () => {
    const panel = document.createElement("div")
    const group = document.createElement("section")
    group.dataset.settingsControl = "checkModelUsage"
    group.dataset.settingsFocusSelf = "true"
    group.tabIndex = -1
    group.scrollIntoView = vi.fn()
    const button = document.createElement("button")
    group.append(button)
    panel.append(group)
    document.body.append(panel)
    new SettingsTargetFocus().attach(panel, "checks", "checkModelUsage", 1)()
    expect(group).toHaveFocus()
    expect(button).not.toHaveFocus()
  })
  it.each(["disabled", "aria-disabled"])("focuses the row when a switch is %s", (attribute) => {
    const panel = document.createElement("div")
    const row = document.createElement("div")
    row.dataset.settingsControl = "notificationSound"
    row.tabIndex = -1
    row.scrollIntoView = vi.fn()
    const button = document.createElement("button")
    button.setAttribute("role", "switch")
    button.setAttribute(attribute, "true")
    row.append(button)
    panel.append(row)
    document.body.append(panel)
    new SettingsTargetFocus().attach(panel, "notifications", "notificationSound", 1)()
    expect(row).toHaveFocus()
  })
  it("finds a delayed target once and does not steal focus on later renders", async () => {
    const focus = new SettingsTargetFocus()
    const panel = document.createElement("div")
    document.body.append(panel)
    const stop = focus.attach(panel, "privacy", "analytics", 1)
    const row = document.createElement("div")
    row.dataset.settingsControl = "analytics"
    row.scrollIntoView = vi.fn()
    const button = document.createElement("button")
    row.append(button)
    panel.append(row)
    await Promise.resolve()
    expect(button).toHaveFocus()
    expect(row.scrollIntoView).toHaveBeenCalledOnce()
    const other = document.createElement("button")
    panel.append(other)
    other.focus()
    stop()
    focus.attach(panel, "privacy", "analytics", 1)()
    expect(other).toHaveFocus()
    focus.attach(panel, "privacy", "analytics", 2)()
    expect(button).toHaveFocus()
  })
  it("waits for busy and disabled changes, then focuses the real input once", async () => {
    const panel = document.createElement("div")
    const region = document.createElement("div")
    region.setAttribute("aria-busy", "true")
    const row = document.createElement("div")
    row.dataset.settingsControl = "typeSafeApiKey"
    row.tabIndex = -1
    row.scrollIntoView = vi.fn()
    const input = document.createElement("input")
    input.disabled = true
    row.append(input)
    region.append(row)
    panel.append(region)
    document.body.append(panel)
    const focus = vi.spyOn(input, "focus")
    const stop = new SettingsTargetFocus().attach(panel, "checks", "typeSafeApiKey", 1)
    expect(row).not.toHaveFocus()
    input.disabled = false
    await Promise.resolve()
    expect(focus).not.toHaveBeenCalled()
    region.setAttribute("aria-busy", "false")
    await Promise.resolve()
    expect(input).toHaveFocus()
    input.disabled = true
    input.disabled = false
    await Promise.resolve()
    expect(focus).toHaveBeenCalledOnce()
    stop()
  })
  it("disconnects an unresolved busy target on teardown", async () => {
    const panel = document.createElement("div")
    panel.setAttribute("aria-busy", "true")
    panel.innerHTML =
      '<div data-settings-control="typeSafeApiKey" tabindex="-1"><input disabled /></div>'
    document.body.append(panel)
    const input = panel.querySelector("input")!
    const focus = vi.spyOn(input, "focus")
    new SettingsTargetFocus().attach(panel, "checks", "typeSafeApiKey", 1)()
    input.disabled = false
    panel.setAttribute("aria-busy", "false")
    await Promise.resolve()
    expect(focus).not.toHaveBeenCalled()
  })
  it("focuses temporary recovery once and keeps the real field pending through Retry", async () => {
    const panel = document.createElement("div")
    const recovery = document.createElement("div")
    recovery.dataset.settingsControl = "customResponseMode"
    recovery.dataset.settingsTargetPending = "true"
    recovery.scrollIntoView = vi.fn()
    const retry = document.createElement("button")
    recovery.append(retry)
    panel.append(recovery)
    document.body.append(panel)
    const focus = new SettingsTargetFocus()
    const stop = focus.attach(panel, "checks", "customResponseMode", 1)
    expect(retry).toHaveFocus()
    const other = document.createElement("button")
    panel.append(other)
    other.focus()
    focus.attach(panel, "checks", "customResponseMode", 1)()
    expect(other).toHaveFocus()
    panel.setAttribute("aria-busy", "true")
    const row = document.createElement("div")
    row.dataset.settingsControl = "customResponseMode"
    row.scrollIntoView = vi.fn()
    const select = document.createElement("select")
    row.append(select)
    recovery.replaceWith(row)
    await Promise.resolve()
    expect(select).not.toHaveFocus()
    panel.setAttribute("aria-busy", "false")
    await Promise.resolve()
    expect(select).toHaveFocus()
    other.focus()
    focus.attach(panel, "checks", "customResponseMode", 1)()
    expect(other).toHaveFocus()
    stop()
  })
})
