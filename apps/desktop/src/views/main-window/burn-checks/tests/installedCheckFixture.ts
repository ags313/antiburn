import type { InstalledCheckReportPayload } from "../../../../lib/insightsIpc"
import report from "./fixtures/installed-check-report.json"

// The Rust report test checks this fixture against the production serializer.
export const installedCheckFixture: InstalledCheckReportPayload = {
  ...report,
  effectiveId: "installed:failed-retry-loop",
  availability: { status: "available" },
}
