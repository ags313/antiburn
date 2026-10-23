import { screen, waitFor } from "@testing-library/react"
import { beforeEach, expect, it, vi } from "vitest"

import { searchApp } from "../../../../lib/appSearch"
import { emptyCheckAvailability } from "../../../../lib/checkAvailability"
import type { InstalledCheckReportPayload } from "../../../../lib/insightsIpc"
import { checkRows } from "../../../../lib/presentation/checkRows"
import { BurnChecksView } from "../../BurnChecksView"
import { report, setup } from "./burnChecksTestSupport"
import { installedCheckFixture } from "./installedCheckFixture"

beforeEach(() => {
  HTMLElement.prototype.scrollIntoView = vi.fn()
})

it("keeps native checks but adds no YAML cards or navigation without discovery", async () => {
  const snapshot = { ...emptyCheckAvailability, configured: false }
  const nativeReport = { ...report, installedChecks: [] }
  setup(null, false, { wins: [] }, nativeReport)
  expect(await screen.findByRole("button", { name: /Old model usage/ })).toBeVisible()
  expect(checkRows(nativeReport)).toEqual(report.categories)
  expect(document.querySelector('[aria-controls^="burn-check-installed:"]')).toBeNull()
  expect(
    searchApp("", "macos", snapshot.configured, snapshot.installedChecks).filter(
      ({ target }) => target.kind === "installedCheck",
    ),
  ).toEqual([])
  expect(
    searchApp("Model-based Checks", "macos", snapshot.configured, snapshot.installedChecks),
  ).toContainEqual(
    expect.objectContaining({
      target: { kind: "setting", control: "smartChecksEnabled" },
    }),
  )
})

it.each([false, true])(
  "shows discovered global and repository checks without a provider when enablement is %s",
  async (yamlEnabled) => {
    const definitions: Pick<InstalledCheckReportPayload, "effectiveId" | "name">[] = [
      { effectiveId: "installed:global-retries", name: "Global retries" },
      { effectiveId: "installed:repo-retries", name: "Repository retries" },
    ]
    const discovered: InstalledCheckReportPayload[] = definitions.map((definition) => ({
      ...installedCheckFixture,
      ...definition,
      aliases: [],
      availability: { status: "paused" },
      finding: 0,
      clean: 0,
      hasStarted: false,
      checkingCount: 0,
      targets: [],
    }))
    const snapshot = {
      ...emptyCheckAvailability,
      yamlEnabled,
      configured: false,
      installedChecks: discovered,
    }
    const mixedReport = { ...report, installedChecks: discovered }
    const { session, view } = setup(null, false, { wins: [] }, mixedReport)
    await screen.findByRole("button", { name: /Old model usage/ })
    expect(checkRows(mixedReport).map(({ id }) => id)).toEqual([
      ...report.categories.map(({ id }) => id),
      ...discovered.map(({ effectiveId }) => effectiveId),
    ])

    for (const [index, check] of discovered.entries()) {
      const result = searchApp(
        check.name,
        "macos",
        snapshot.configured,
        snapshot.installedChecks,
      )[0]!
      expect(result.target).toEqual({ kind: "installedCheck", check: check.effectiveId })
      view.rerender(
        <BurnChecksView
          active
          session={session}
          focusedCheck={check.effectiveId}
          focusRevision={index + 1}
        />,
      )
      const row = await screen.findByRole("button", { name: `${check.name}, Paused` })
      expect(row).toBeVisible()
      await waitFor(() => expect(row).toHaveFocus())
      expect(screen.getByRole("heading", { name: check.name })).toBeVisible()
    }
  },
)
