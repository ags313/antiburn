import { expect, test } from "@playwright/test"

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.removeItem("antiburn.promotions.seen.v1")
    const age = new URLSearchParams(location.search).get("promoAge")
    if (age === null) localStorage.removeItem("antiburn.promotions.ftue.v1")
    else
      localStorage.setItem(
        "antiburn.promotions.ftue.v1",
        JSON.stringify({ completedAt: Date.now() - Number(age) }),
      )
  })
})

for (const theme of ["light", "dark"] as const) {
  test(`${theme}: automatic promo, fixed frame, no scrolling and cover media`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ colorScheme: theme })
    await page.goto(
      `/tests/visual/?surface=main&theme=${theme}&scale=100&state=populated&onboarding=complete`,
    )
    await expect(page.locator(".main-window")).toBeVisible({ timeout: 20_000 })
    await page.bringToFront()
    const panel = page.locator(".promotion-panel")
    await expect(panel).toBeVisible()
    const shadows = await panel.evaluate((node, theme) => {
      document.documentElement.dataset.theme = theme
      const explicit = getComputedStyle(node).boxShadow
      delete document.documentElement.dataset.theme
      const system = getComputedStyle(node).boxShadow
      document.documentElement.dataset.theme = theme
      return { explicit, system }
    }, theme)
    expect(shadows.system).toBe(shadows.explicit)
    expect(shadows.explicit).toBe(
      theme === "dark"
        ? "rgba(0, 0, 0, 0.32) 6px 0px 18px 0px, rgba(0, 0, 0, 0.18) 0px 2px 6px 0px"
        : "rgba(0, 0, 0, 0.15) 0px 4px 12px 0px, rgba(0, 0, 0, 0.08) 0px 1px 3px 0px",
    )
    await expect(page.getByRole("button", { name: "What’s new" })).toBeHidden()
    await expect(page.locator(".main-window-sidebar .sidebar-nav-footer")).toHaveCSS(
      "visibility",
      "hidden",
    )
    await expect(panel).not.toHaveCSS("box-shadow", "none")
    await expect(panel).toHaveCSS("transform", "none")
    const before = await panel.boundingBox()
    expect(before?.width).toBe(320)
    expect(before?.height).toBe(440)
    expect(before?.x).toBe(8)
    expect(before!.y + before!.height).toBe(860 - 8)
    await expect(panel.locator(".promotion-primary-fill")).toHaveCSS("min-height", "28px")
    await expect(panel.locator(".promotion-primary")).toHaveCSS("min-height", "40px")
    expect(
      await panel.evaluate((node) => getComputedStyle(node, "::after").boxShadow),
    ).not.toBe("none")
    const overflow = () =>
      page
        .locator(".promotion-scroll")
        .evaluate((node) => node.scrollHeight - node.clientHeight)
    expect(await overflow()).toBeLessThanOrEqual(1)
    expect(
      await page
        .locator(".promotion-media img")
        .evaluate((node) => getComputedStyle(node).objectFit),
    ).toBe("cover")
    const primary = page.getByRole("button", { name: "Set up" })
    const contrast = () =>
      primary.evaluate((node) => {
        const style = getComputedStyle(node.querySelector(".promotion-primary-fill")!)
        const canvas = document.createElement("canvas")
        canvas.width = canvas.height = 1
        const context = canvas.getContext("2d")!
        context.fillStyle = style.backgroundColor
        context.fillRect(0, 0, 1, 1)
        const background = [...context.getImageData(0, 0, 1, 1).data].slice(0, 3)
        context.fillStyle = style.color
        context.fillRect(0, 0, 1, 1)
        const foreground = [...context.getImageData(0, 0, 1, 1).data].slice(0, 3)
        const luminance = (rgb: number[]) =>
          rgb
            .map((value) => {
              const channel = value / 255
              return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
            })
            .reduce((sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index]!, 0)
        const values = [luminance(background), luminance(foreground)].sort((a, b) => b - a)
        return { ratio: (values[0]! + 0.05) / (values[1]! + 0.05), fill: style.backgroundColor }
      })
    const normal = await contrast()
    expect(normal.ratio).toBeGreaterThanOrEqual(4.5)
    await expect(primary).toHaveCSS("color", "rgb(255, 255, 255)")
    await primary.focus()
    await expect.poll(async () => (await contrast()).fill).not.toBe(normal.fill)
    expect((await contrast()).ratio).toBeGreaterThanOrEqual(4.5)
    await primary.hover()
    await expect.poll(async () => (await contrast()).fill).not.toBe(normal.fill)
    expect((await contrast()).ratio).toBeGreaterThanOrEqual(4.5)
    const hovered = await contrast()
    await page.mouse.down()
    await expect.poll(async () => (await contrast()).fill).not.toBe(hovered.fill)
    expect((await contrast()).ratio).toBeGreaterThanOrEqual(4.5)
    await page.mouse.move(600, 40)
    await page.mouse.up()
    await page.screenshot({ path: testInfo.outputPath(`${theme}-announcement.png`) })
    await page.getByRole("button", { name: "Set up" }).click()
    expect(await panel.boundingBox()).toEqual(before)
    await expect(panel.locator(".promotion-step-number").first()).not.toHaveCSS(
      "border-radius",
      "0px",
    )
    expect(await overflow()).toBeLessThanOrEqual(1)
    await expect(page.getByText(/Run claude in terminal or desktop/)).toBeVisible()
    await page.screenshot({ path: testInfo.outputPath(`${theme}-setup.png`) })
    for (const step of await panel.locator(".promotion-steps > li").all()) {
      const command = step.locator(".promotion-command")
      if ((await command.count()) === 0) continue
      const commandBox = await command.boundingBox()
      const faceBox = await step.locator(".promotion-copy-fill").boundingBox()
      expect(faceBox!.x + faceBox!.width).toBeCloseTo(commandBox!.x + commandBox!.width, 1)
    }
    const copyButton = page.getByRole("button", { name: "Copy Install Claude Mod" })
    await copyButton.hover()
    await expect(copyButton).toHaveCSS("background-color", "rgba(0, 0, 0, 0)")
    const copyFill = copyButton.locator(".promotion-copy-fill")
    await expect(copyFill).not.toHaveCSS("background-color", "rgba(0, 0, 0, 0)")
    const copyFace = await copyFill.boundingBox()
    expect(copyFace?.width).toBe(28)
    expect(copyFace?.height).toBe(28)
    const copyTarget = await copyButton.boundingBox()
    expect(copyTarget?.width).toBe(60)
    expect(copyTarget?.height).toBe(40)
    await page.getByRole("button", { name: "Copy Install Claude Mod" }).click()
    await expect(page.getByRole("button", { name: "Copied Install Claude Mod" })).toHaveText(
      "Copied!",
    )
    await page.getByRole("button", { name: "Close promotion" }).click()
    await expect(panel).toBeHidden()
    await expect(page.getByRole("button", { name: "What’s new" })).toHaveCount(0)
  })
}

test("reduced motion and failed animation retain a full-bleed accessible poster", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce", colorScheme: "dark" })
  await page.goto(
    "/tests/visual/?surface=main&theme=dark&scale=100&state=populated&onboarding=complete",
  )
  const media = page.locator(".promotion-media")
  const before = await media.boundingBox()
  await expect(media).toHaveAttribute("role", "img")
  await expect(media).toHaveAttribute("aria-label", /.+/)
  const image = media.locator("img")
  await expect(image).toHaveAttribute("src", /claude-mod-poster/)
  await page.emulateMedia({ reducedMotion: "no-preference" })
  await expect(image).toHaveAttribute("src", /claude-mod-promo.*\.gif/)
  await image.dispatchEvent("error")
  await expect(image).toHaveAttribute("src", /claude-mod-poster/)
  await expect(media.locator("img")).toBeVisible()
  await expect(media.locator("img")).toHaveCSS("object-fit", "cover")
  expect(await media.boundingBox()).toEqual(before)
})

test("small viewport keeps the panel and footer reachable", async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 480 })
  await page.goto(
    "/tests/visual/?surface=main&theme=dark&scale=100&state=populated&onboarding=complete",
  )
  await page.getByRole("button", { name: "Set up" }).click()
  const box = await page.locator(".promotion-panel").boundingBox()
  expect(box!.y).toBeGreaterThanOrEqual(0)
  expect(box!.y + box!.height).toBeLessThanOrEqual(480)
  await expect(page.getByRole("button", { name: "Learn more" })).toBeVisible()
})

test("new users wait 24 hours after FTUE before automatic display", async ({ page }) => {
  await page.goto(
    "/tests/visual/?surface=main&theme=dark&state=populated&onboarding=complete&promoAge=0",
  )
  await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeEnabled()
  await expect(page.locator(".promotion-panel")).toHaveCount(0)
  expect(
    await page.evaluate(() => localStorage.getItem("antiburn.promotions.seen.v1")),
  ).toBeNull()
  await page.goto(
    "/tests/visual/?surface=main&theme=dark&state=populated&onboarding=complete&promoAge=86400000",
  )
  await expect(page.locator(".promotion-panel")).toBeVisible()
})

test("MAIN-window FTUE never displays or marks a promo seen", async ({ page }) => {
  await page.goto("/tests/visual/?surface=main&theme=dark&state=populated")
  await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeDisabled()
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("antiburn.promotions.ftue.v1")))
    .not.toBeNull()
  await expect(page.locator(".promotion-panel")).toHaveCount(0)
  expect(
    await page.evaluate(() => localStorage.getItem("antiburn.promotions.seen.v1")),
  ).toBeNull()
})

for (const reduced of [false, true]) {
  test(`promo reveal runs once per opening (reduced motion: ${reduced})`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: reduced ? "reduce" : "no-preference" })
    await page.addInitScript(() => {
      const animate = Element.prototype.animate
      Element.prototype.animate = function (keyframes, options) {
        const animation = animate.call(this, keyframes, options)
        if (this.classList.contains("promotion-panel")) {
          const previous = JSON.parse(sessionStorage.getItem("promo-reveal-test") ?? "null")
          sessionStorage.setItem(
            "promo-reveal-test",
            JSON.stringify({
              keyframes,
              options,
              count: (previous?.count ?? 0) + 1,
              status: "running",
            }),
          )
        }
        if (this.classList.contains("promotion-panel")) {
          for (const event of ["finish", "cancel"]) {
            animation.addEventListener(event, () => {
              const current = JSON.parse(sessionStorage.getItem("promo-reveal-test")!)
              current.status = event
              sessionStorage.setItem("promo-reveal-test", JSON.stringify(current))
            })
          }
        }
        return animation
      }
    })
    await page.goto(
      "/tests/visual/?surface=main&theme=dark&state=populated&onboarding=complete",
    )
    const panel = page.locator(".promotion-panel")
    await expect(panel).toBeVisible()
    const reveal = () =>
      page.evaluate(() => JSON.parse(sessionStorage.getItem("promo-reveal-test") ?? "null"))
    const initial = await reveal()
    expect(initial.count).toBe(1)
    expect(initial.options.duration).toBe(reduced ? 100 : 180)
    expect(initial.options.easing).toBe("cubic-bezier(0.23, 1, 0.32, 1)")
    expect(initial.keyframes).toEqual(
      reduced
        ? [{ opacity: 0 }, { opacity: 1 }]
        : [
            { opacity: 0, transform: "translateY(8px)" },
            { opacity: 1, transform: "translateY(0)" },
          ],
    )
    await expect.poll(async () => (await reveal()).status).toBe("finish")
    await expect(panel).toHaveCSS("opacity", "1")
    await expect(panel).toHaveCSS("transform", "none")
    await page.getByRole("button", { name: "Set up", exact: true }).click()
    await panel.getByRole("button", { name: "Back", exact: true }).click()
    await page.evaluate(() => {
      window.dispatchEvent(new Event("blur"))
      window.dispatchEvent(new Event("focus"))
    })
    expect((await reveal()).count).toBe(1)
  })
}

test("light v14 video and poster switch together while dark media stays unchanged", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "light" })
  await page.goto(
    "/tests/visual/?surface=main&theme=light&scale=100&state=populated&onboarding=complete",
  )
  const panel = page.locator(".promotion-panel")
  await expect(panel).toBeVisible()
  const video = panel.locator("video")
  await expect(video).toHaveAttribute("src", /claude-mod-light-v14.*\.mp4/)
  await expect(video).toHaveAttribute("poster", /claude-mod-light-v14-poster.*\.png/)
  await expect(panel.locator(".promotion-poster")).toHaveAttribute(
    "src",
    /claude-mod-light-v14-poster.*\.png/,
  )
  await expect
    .poll(() => video.evaluate((node: HTMLVideoElement) => node.currentTime))
    .toBeGreaterThan(0)
  expect(
    await video.evaluate((node: HTMLVideoElement) => ({
      width: node.videoWidth,
      height: node.videoHeight,
      muted: node.muted,
      error: node.error?.code ?? null,
    })),
  ).toEqual({ width: 800, height: 416, muted: true, error: null })
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark"
  })
  await expect(panel.locator("video")).toHaveCount(0)
  await expect(panel.locator(".promotion-media img")).toHaveAttribute(
    "src",
    /claude-mod-promo.*\.gif/,
  )
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "light"
  })
  await expect(video).toHaveAttribute("poster", /claude-mod-light-v14-poster.*\.png/)
  await page.emulateMedia({ reducedMotion: "reduce" })
  await expect(video).toHaveCSS("visibility", "hidden")
  await expect.poll(() => video.evaluate((node: HTMLVideoElement) => node.paused)).toBe(true)
  await expect(panel.locator(".promotion-poster")).toBeVisible()
  await page.evaluate(() => {
    delete document.documentElement.dataset.theme
  })
  await page.emulateMedia({ colorScheme: "dark" })
  await expect(panel.locator("video")).toHaveCount(0)
  await expect(panel.locator(".promotion-media img")).toHaveAttribute(
    "src",
    /claude-mod-poster.*\.png/,
  )
})
