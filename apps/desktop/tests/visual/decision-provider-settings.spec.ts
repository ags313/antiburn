import { expect, test } from "@playwright/test"

for (const theme of ["light", "dark"] as const) {
  for (const layout of [
    { name: "normal", width: 1100, height: 800, scale: 100 },
    { name: "constrained", width: 640, height: 600, scale: 200 },
  ]) {
    test(`model-based check settings stay readable: ${theme}, ${layout.name}`, async ({
      page,
    }, testInfo) => {
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
      const pane = page.getByRole("tabpanel", { name: "Checks", exact: true })
      await expect(pane.getByRole("heading", { level: 2 })).toHaveText([
        "Local checks",
        "Model-based checks",
      ])
      await expect(page.getByRole("heading", { name: "Decision model" })).toHaveCount(0)
      const heading = pane.getByRole("heading", { name: "Model-based checks", exact: true })
      const section = pane.locator("section").filter({
        has: page.getByRole("heading", { name: "Model-based checks", exact: true }),
      })
      await expect(section).toHaveCount(1)
      await expect(
        section.getByRole("switch", { name: "Run model-based checks", exact: true }),
      ).toBeEnabled()
      const provider = page.getByLabel("Provider", { exact: true })
      await expect(provider).toBeEnabled()
      await expect(section.getByText(/To install a check/)).toContainText(
        "put a .yaml or .yml file directly in either folder:",
      )
      const paths = section.locator("dl")
      await expect(paths.getByRole("term")).toHaveText(["This device", "Known project"])
      await expect(paths.getByRole("definition")).toHaveText([
        "~/.antiburn/checks/",
        ".antiburn/checks/",
      ])
      await expect(
        section.getByText("Project folders must belong to known native projects.", {
          exact: true,
        }),
      ).toBeVisible()
      for (const path of await paths.getByRole("definition").all()) {
        expect(
          await path.evaluate((node) => node.scrollWidth - node.clientWidth),
        ).toBeLessThanOrEqual(1)
        await expect(path.locator("code")).toHaveCSS("overflow-wrap", "anywhere")
      }
      await heading.evaluate((node) => node.scrollIntoView({ block: "start" }))
      await page.evaluate(() => document.fonts.ready)
      await page.screenshot({
        path: testInfo.outputPath(`checks-install-${theme}-${layout.scale}.png`),
        animations: "disabled",
      })
      await expect(
        page.getByRole("button", { name: "Model limits", exact: true }),
      ).toHaveAttribute("aria-expanded", "false")
      for (const [route, control, label] of [
        ["jev", "typeSafeApiKey", "Jev API key"],
        ["ollama", "ollamaUrl", "Ollama URL"],
        ["cloudflare", "cloudflareAccountId", "Cloudflare account ID"],
        ["custom", "customUrl", "Exact inference URL"],
      ] as const) {
        await provider.selectOption(route)
        const field = page.getByLabel(label, { exact: true })
        const value = await field.inputValue()
        await page.evaluate((target) => {
          window.__ANTIBURN_VISUAL_EMIT__?.("settings:pane", `checks#${target}`)
        }, control)
        await expect(field).toBeFocused()
        await expect(field).toHaveValue(value)
        await expect(field).toHaveAccessibleName(label)
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
        const insets = await section.evaluate((node) => {
          const selectedField = node.querySelector(
            "label[data-settings-control] input, label[data-settings-control] select",
          )
          const fieldGroup = selectedField?.closest("label")?.parentElement
          const limitsIcon = node.querySelector(
            '[data-settings-control="decisionModelLimits"] svg',
          )
          const saveButton = [...node.querySelectorAll("button")].find(
            (button) => button.textContent === "Save and use",
          )
          const actions = saveButton?.parentElement?.parentElement
          const usageButton = [...node.querySelectorAll("button")].find(
            (button) => button.textContent === "Model usage",
          )
          const usageIcon = usageButton?.querySelector("svg")
          if (!fieldGroup || !limitsIcon || !actions || !usageIcon)
            throw new Error("Missing provider layout sections")
          const style = getComputedStyle(fieldGroup)
          return {
            fieldLeft: selectedField!.closest("label")!.getBoundingClientRect().left,
            limitsLeft: limitsIcon.getBoundingClientRect().left,
            actionsLeft: actions.querySelector("p")!.getBoundingClientRect().left,
            usageLeft: usageIcon.getBoundingClientRect().left,
            paddingTop: style.paddingTop,
            paddingBottom: style.paddingBottom,
          }
        })
        expect(Math.abs(insets.fieldLeft - insets.limitsLeft)).toBeLessThanOrEqual(1)
        expect(Math.abs(insets.fieldLeft - insets.actionsLeft)).toBeLessThanOrEqual(1)
        expect(Math.abs(insets.fieldLeft - insets.usageLeft)).toBeLessThanOrEqual(1)
        expect(insets.paddingTop).toBe(insets.paddingBottom)
        expect(Number.parseFloat(insets.paddingTop)).toBeGreaterThan(0)
        await field.locator("..").evaluate((node) => node.scrollIntoView({ block: "start" }))
        await page.screenshot({
          path: testInfo.outputPath(`checks-${route}-fields-${theme}-${layout.scale}.png`),
          animations: "disabled",
        })
        await page.getByRole("button", { name: "Model limits", exact: true }).click()
        for (const label of [
          "Total input tokens",
          "State and longest question tokens",
          "Loaded context tokens",
        ]) {
          const limit = page.getByLabel(label, { exact: true })
          await limit.scrollIntoViewIfNeeded()
          await expect(limit).toBeVisible()
          await expect(limit).toHaveAttribute("min", "1")
          await expect(limit).toHaveAttribute("max", "65536")
        }
        const tokens = page.getByLabel("Loaded context tokens", { exact: true })
        await tokens.fill("8192")
        await expect(
          page.getByText("Unsaved draft. Save and use to activate it."),
        ).toBeVisible()
        await expect(page.getByText(/^Active:/)).toContainText("Jev")
        await expect(page.getByLabel("Saved connection", { exact: true })).toHaveValue(route)
        await expect(
          page.getByText(
            "Tests use synthetic input and do not activate drafts. Provider charges may apply.",
          ),
        ).toBeVisible()
        expect(
          await pane.evaluate((node) => node.scrollWidth - node.clientWidth),
        ).toBeLessThanOrEqual(1)
        await tokens.locator("..").evaluate((node) => node.scrollIntoView({ block: "center" }))
        await page.screenshot({
          path: testInfo.outputPath(`checks-${route}-limits-${theme}-${layout.scale}.png`),
          animations: "disabled",
        })
        await page.getByRole("button", { name: "Model limits", exact: true }).click()
      }
    })
  }
}
