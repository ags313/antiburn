import { expect, test } from "@playwright/test"

for (const theme of ["light", "dark"] as const) {
  test(`Cost stays readable when entrances are suspended in ${theme}`, async ({
    page,
  }, testInfo) => {
    await page.clock.setFixedTime(new Date("2026-09-15T00:00:00Z"))
    await page.emulateMedia({ colorScheme: theme })
    await page.goto(
      `/tests/visual/?surface=main&scale=100&theme=${theme}&state=populated&onboarding=complete&spend=populated`,
    )
    await expect(page.getByRole("radio", { name: "Cost", exact: true })).toBeVisible()
    await page.addStyleTag({ content: "* { animation-play-state: paused !important; }" })
    await page.getByRole("radio", { name: "Cost", exact: true }).click()
    const totals = page.getByRole("region", { name: "Estimated local spend" })
    await expect(totals).toBeVisible()
    await expect(totals).toHaveCSS("opacity", "1")
    await expect(totals.locator(".type-hero-figure").first()).toBeInViewport()
    const plot = page.locator('[data-usage-banner="spend"] svg')
    await expect.poll(async () => (await plot.boundingBox())?.height ?? 0).toBeGreaterThan(10)
    await expect(plot.locator("rect").first()).toBeInViewport()
    await page.screenshot({ path: testInfo.outputPath(`cost-suspended-${theme}.png`) })
  })
}
