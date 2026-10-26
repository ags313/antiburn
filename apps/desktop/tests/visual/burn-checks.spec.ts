import { expect, test } from "@playwright/test"

for (const theme of ["light", "dark"] as const) {
  for (const width of [1280, 640]) {
    test(`installed check results stay clear in ${theme} at ${width}`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width, height: width === 640 ? 480 : 860 })
      await page.clock.setFixedTime(new Date("2026-09-15T00:00:00Z"))
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" })
      await page.goto(
        `/tests/visual/?surface=main&onboarding=complete&checks=installed&theme=${theme}`,
      )
      const overview = page.getByRole("tabpanel", { name: "Overview", exact: true })
      await expect(overview).toBeVisible({ timeout: 30_000 })
      await expect(
        overview.getByRole("button", { name: "First assessment, Not assessed · Checking" }),
      ).toBeVisible()
      await expect(
        overview.getByRole("button", { name: "First assessment, Passed · Checking" }),
      ).toHaveCount(0)
      await expect(
        overview.getByRole("button", {
          name: "Connection unavailable, Not assessed",
        }),
      ).toBeVisible()

      const navigation = page.getByRole("button", { name: "Open Main navigation" })
      if (await navigation.isVisible()) await navigation.click()
      await page.getByRole("tab", { name: "Checks", exact: true }).click()
      const collection = page.getByRole("region", { name: "Burn check collection" })
      const failed = collection.getByRole("button", {
        name: "Saved failure, 1 failed · Checking",
      })
      const passed = collection.getByRole("button", { name: "Saved pass, Passed · Checking" })
      await expect(collection.getByRole("region", { name: "Failed checks 1" })).toContainText(
        "Saved failure",
      )
      await expect(collection.getByRole("region", { name: "Passed checks 1" })).toContainText(
        "Saved pass",
      )
      await expect(failed).toHaveAttribute("data-outcome", "failed")
      await expect(passed).toHaveAttribute("data-outcome", "passed")
      await expect(collection.getByRole("region", { name: /^Checking/ })).toHaveCount(0)

      const neutralGroup = collection.getByRole("button", { name: "Not assessed (3)" })
      await neutralGroup.focus()
      await neutralGroup.press("ArrowDown")
      const first = collection.getByRole("button", {
        name: "First assessment, Not assessed · Checking",
      })
      await expect(first).toBeFocused()
      await expect(first).toHaveAttribute("data-outcome", "unassessed")
      await expect(first).not.toContainText("Passed")
      await first.press("Enter")
      const details = page.getByRole("region", { name: "Burn check details" })
      await expect(details.getByRole("status")).toHaveText(
        "First assessment: Not assessed · Checking",
      )
      await collection
        .getByRole("button", { name: "Waiting for activity, Not assessed" })
        .click()
      await expect(
        details.getByText(
          "This check is waiting for new tool results. It needs 3 more tool results before it can assess session activity.",
        ),
      ).toBeVisible()
      await collection
        .getByRole("button", { name: "Connection unavailable, Not assessed" })
        .click()
      await expect(
        details.getByRole("region", { name: "Decision connection needed" }),
      ).toBeVisible()
      await expect(details.getByRole("status")).toHaveText(
        "Connection unavailable: Not assessed. Decision connection needed",
      )

      await failed.click()
      const alert = details.getByRole("region", {
        name: "OpenCode live checks are unavailable",
      })
      await expect(alert).toBeVisible()
      await expect(
        details.getByRole("region", { name: "OpenCode live checks are unavailable" }),
      ).toHaveCount(1)
      await expect(
        details.getByText("The same failed command ran again without a change."),
      ).toBeVisible()
      await expect(details.getByText(/OpenCode V2|Missing information|missed/i)).toHaveCount(0)
      const technical = alert.locator("summary", { hasText: "Technical details" })
      await expect(technical.locator("..")).not.toHaveAttribute("open", "")
      await technical.focus()
      await technical.press("Enter")
      await expect(alert.getByText("opencode-v2-capture-gap", { exact: true })).toBeVisible()
      await technical.press("Enter")
      await expect(
        alert.getByText("opencode-v2-capture-gap", { exact: true }),
      ).not.toBeVisible()
      const moreIssues = details.locator("summary", { hasText: "2 more issues" })
      await expect(moreIssues).toBeVisible()
      await expect(moreIssues.locator("..")).not.toHaveAttribute("open", "")
      await alert.scrollIntoViewIfNeeded()
      await page.screenshot({
        path: testInfo.outputPath(`burn-checks-${theme}-${width}.png`),
        animations: "disabled",
      })
      await moreIssues.focus()
      await moreIssues.press("Enter")
      await expect(
        details.getByRole("region", { name: "Session journal unavailable" }),
      ).toBeVisible()
      await expect(
        details.getByRole("region", { name: "Session source unavailable" }),
      ).toBeVisible()
      await expect(technical.locator("..")).not.toHaveAttribute("open", "")
      await page.screenshot({
        path: testInfo.outputPath(`burn-checks-more-issues-${theme}-${width}.png`),
        animations: "disabled",
      })
      expect(
        await details.evaluate((node) => node.scrollWidth - node.clientWidth),
      ).toBeLessThanOrEqual(1)
    })
  }
}
