import { describe, expect, it } from "vitest"
import type { InstalledCheckDiagnosticPayload } from "../insightsIpc"
import { installedCheckErrorStatus } from "./installedCheckDiagnostics"

function sourceDiagnostic(
  severity: InstalledCheckDiagnosticPayload["severity"],
): InstalledCheckDiagnosticPayload {
  return {
    code: "opencode-v2-capture-gap",
    severity,
    title: "Source history is incomplete",
    message: "Earlier results are unavailable.",
    source: null,
    details: [],
    affectedSessionCount: 1,
  }
}

describe("installed check diagnostic labels", () => {
  it("leaves an informational source limit out of the error status", () => {
    expect(installedCheckErrorStatus([sourceDiagnostic("info")])).toBeNull()
  })

  it("labels an error without deriving aggregate session state", () => {
    expect(installedCheckErrorStatus([sourceDiagnostic("error")])).toBe("Missing information")
  })
})
