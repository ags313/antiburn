import { expect, test } from "@playwright/test"
import type { CheckAvailability } from "../../src/lib/checkAvailability"

const removedAvailability: CheckAvailability = {
  revision: 1,
  yamlEnabled: true,
  installedChecks: [],
  checks: [],
  configured: false,
  savedKey: false,
  error: null,
  usage: {
    inputTokens: 0,
    outputTokens: 0,
    confirmedCalls: 0,
    cacheHits: 0,
    unknownOutcomes: 0,
    estimatedUsd: "$0.00",
    lastUsedAtEpoch: null,
  },
}

for (const keepSearchOpen of [false, true]) {
  test(`removed definitions leave search and details${keepSearchOpen ? " without taking later focus" : ""}`, async ({
    page,
  }) => {
    await page.goto(
      "/tests/visual/?surface=main&onboarding=complete&checks=installed&theme=light",
    )
    const search = page.getByRole("button", { name: "Search antiburn", exact: true })
    await expect(search).toBeVisible({ timeout: 30_000 })
    await search.click()
    const dialog = page.getByRole("dialog", { name: "Search antiburn" })
    const query = dialog.getByRole("combobox", { name: "Search antiburn" })
    await query.fill("Saved failure")
    await expect(dialog.getByRole("option", { name: /Saved failure/ })).toBeVisible()
    await query.press("Enter")
    await expect(dialog).toHaveCount(0)
    const details = page.locator('[role="region"][aria-label="Burn check details"]')
    await expect(
      details.getByRole("heading", { name: "Saved failure", exact: true }),
    ).toBeVisible()
    if (keepSearchOpen) {
      await search.click()
      await query.fill("Saved failure")
      await expect(query).toBeFocused()
    }
    await page.evaluate((snapshot) => {
      const url = new URL(window.location.href)
      url.searchParams.set("checks", "removed")
      history.replaceState(null, "", url)
      window.__ANTIBURN_VISUAL_EMIT__?.("checks:availability-changed", {
        status: "updated",
        snapshot,
      })
      window.__ANTIBURN_VISUAL_EMIT__?.("checks:report-changed", undefined)
    }, removedAvailability)
    await expect(
      page.getByRole("button", {
        name: "Saved failure, 1 failed · Checking",
        includeHidden: true,
      }),
    ).toHaveCount(0)
    await expect(
      details.getByRole("heading", { name: "Saved failure", exact: true, includeHidden: true }),
    ).toHaveCount(0)
    await expect(
      details.getByText("The same failed command ran again without a change.", { exact: true }),
    ).toHaveCount(0)
    if (keepSearchOpen) await expect(query).toBeFocused()
    else {
      expect(await page.evaluate(() => document.activeElement === document.body)).toBe(false)
      await search.click()
      await query.fill("Saved failure")
    }
    await expect(dialog.getByRole("option", { name: /Saved failure/ })).toHaveCount(0)
    await expect(dialog.getByText("No matching destinations.", { exact: true })).toBeVisible()
    await query.press("Escape")
    await expect(dialog).toHaveCount(0)
    await expect(
      details.getByRole("heading", { name: "Saved failure", exact: true }),
    ).toHaveCount(0)
  })
}
