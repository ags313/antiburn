import { expect, test } from "@playwright/test"

for (const theme of ["light", "dark"] as const) {
  for (const layout of [
    { name: "normal", width: 1100, height: 800, scale: 100 },
    { name: "constrained", width: 640, height: 600, scale: 200 },
  ]) {
    test(`decision connections stay simple and reachable: ${theme}, ${layout.name}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: layout.width, height: layout.height })
      await page.emulateMedia({ colorScheme: theme })
      await page.goto(`/tests/visual/?surface=settings&theme=${theme}&scale=${layout.scale}`)
      await expect(page.getByRole("heading", { name: "General", exact: true })).toBeVisible()
      await page.locator(".fixture-scale-root").evaluate((node, scale) => {
        if (!(node instanceof HTMLElement)) throw new Error("Missing fixture root")
        node.style.zoom = String(scale / 100)
      }, layout.scale)
      const navigation = page.getByRole("button", { name: "Open Settings navigation" })
      if (await navigation.isVisible()) await navigation.click()
      await page.getByRole("tab", { name: "Checks", exact: true }).click()
      const provider = page.getByLabel("Provider", { exact: true })
      await expect(provider).toBeEnabled()
      await expect(
        page.getByRole("button", { name: "Model limits", exact: true }),
      ).toHaveAttribute("aria-expanded", "false")
      for (const [route, label] of [
        ["jev", "TypeSafe API key"],
        ["ollama", "Ollama URL"],
        ["cloudflare", "Cloudflare account ID"],
        ["custom", "Exact inference URL"],
      ] as const) {
        await provider.selectOption(route)
        const field = page.getByLabel(label, { exact: true })
        await field.scrollIntoViewIfNeeded()
        await expect(field).toBeVisible()
        if (route === "cloudflare")
          await page.getByLabel("Cloudflare model", { exact: true }).selectOption("clef-flash")
        if (route === "custom")
          await page
            .getByLabel("Response mode", { exact: true })
            .selectOption("cloudflare_envelope")
        await expect(page.locator("#root")).toHaveJSProperty(
          "scrollWidth",
          await page.locator("#root").evaluate((node) => node.clientWidth),
        )
        expect(
          await field.evaluate((node) => {
            const bounds = node.getBoundingClientRect()
            return bounds.left >= 0 && bounds.right <= document.documentElement.clientWidth
          }),
        ).toBe(true)
        await expect(page.getByText(/^Active:/)).toContainText("Jev")
        expect(
          await page
            .locator(
              "[data-settings-control] input, [data-settings-control] select, [data-settings-control] button",
            )
            .evaluateAll((nodes) =>
              nodes
                .filter((node) => {
                  const bounds = node.getBoundingClientRect()
                  return (
                    bounds.width > 0 &&
                    (bounds.left < 0 || bounds.right > document.documentElement.clientWidth)
                  )
                })
                .map((node) => node.getAttribute("aria-label") ?? node.textContent),
            ),
        ).toEqual([])
      }
      await page.getByRole("button", { name: "Model limits", exact: true }).click()
      const tokens = page.getByLabel("Loaded context tokens", { exact: true })
      await tokens.scrollIntoViewIfNeeded()
      await expect(tokens).toBeVisible()
      await tokens.fill("8192")
      await expect(page.getByText("Unsaved draft. Save and use to activate it.")).toBeVisible()
    })
  }
}
