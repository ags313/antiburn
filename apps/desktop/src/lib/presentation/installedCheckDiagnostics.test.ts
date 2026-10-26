import { describe, expect, it } from "vitest"
import { installedCheckDiagnosticFixture as diagnostic } from "../../views/main-window/burn-checks/tests/installedCheckFixture"
import { installedCheckDiagnostics } from "./installedCheckDiagnostics"

describe("installed diagnostic grouping", () => {
  it.each([false, true])(
    "keeps one error for the same issue with error first=%s",
    (errorFirst) => {
      const error = diagnostic
      const info = {
        ...diagnostic,
        severity: "info" as const,
        details: [...diagnostic.details].reverse(),
      }
      expect(installedCheckDiagnostics(errorFirst ? [error, info] : [info, error])).toEqual([
        error,
      ])
    },
  )

  it("does not merge different files, lines, decisions, or source issues", () => {
    const diagnostics = [
      diagnostic,
      { ...diagnostic, source: { ...diagnostic.source!, path: "/workspace/other/check.yaml" } },
      { ...diagnostic, source: { ...diagnostic.source!, line: 30 } },
      { ...diagnostic, details: [{ label: "Decision", value: "recovery" }] },
      { ...diagnostic, code: "source-unavailable" },
    ]
    expect(installedCheckDiagnostics(diagnostics)).toEqual(diagnostics)
  })

  it("keeps errors before unrelated notices", () => {
    const notice = { ...diagnostic, severity: "info" as const, code: "input-shortened" }
    expect(installedCheckDiagnostics([notice, diagnostic])).toEqual([diagnostic, notice])
  })

  it.each([
    { title: "Session journal unavailable", message: "Rescan sessions to restore the source." },
    { title: "Session journal unavailable", message: "Restore access to the session journal." },
    { title: "Session source unavailable", message: "Restore access to the session source." },
    {
      title: "Session source unavailable",
      message: "The session source is no longer available.",
    },
  ])("does not merge distinct source-unavailable copy: $title, $message", (copy) => {
    const first = {
      ...diagnostic,
      code: "source-unavailable",
      title: "Session source unavailable",
      message: "Rescan sessions to restore the source.",
    }
    const second = { ...first, ...copy }
    expect(installedCheckDiagnostics([first, second])).toEqual([first, second])
  })
})
