import type { InstalledCheckDiagnosticPayload } from "../insightsIpc"

export function installedCheckDiagnosticKey(
  diagnostic: InstalledCheckDiagnosticPayload,
): string {
  return JSON.stringify([
    diagnostic.code,
    diagnostic.title,
    diagnostic.message,
    diagnostic.source && [
      diagnostic.source.path,
      diagnostic.source.line,
      diagnostic.source.column,
    ],
    diagnostic.details
      .filter((detail) => detail.label !== "Diagnostic code")
      .map((detail): [string, string] => [detail.label, detail.value])
      .sort(
        ([leftLabel, leftValue], [rightLabel, rightValue]) =>
          leftLabel.localeCompare(rightLabel) || leftValue.localeCompare(rightValue),
      ),
  ])
}

export function installedCheckDiagnostics(
  diagnostics: readonly InstalledCheckDiagnosticPayload[],
): InstalledCheckDiagnosticPayload[] {
  const issues = new Map<string, InstalledCheckDiagnosticPayload>()
  for (const diagnostic of diagnostics) {
    const key = installedCheckDiagnosticKey(diagnostic)
    const previous = issues.get(key)
    if (!previous || (previous.severity === "info" && diagnostic.severity === "error"))
      issues.set(key, diagnostic)
  }
  return [...issues.values()].sort(
    (left, right) => Number(left.severity === "info") - Number(right.severity === "info"),
  )
}
