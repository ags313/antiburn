import type { EngineInterface, SessionAuthorization } from 'claude-code'
import { normalizeUsage, type RateLimitReading, type UsageSnapshot } from './usage'

export type AccountUsageResult =
  { status: 'ready'; snapshot: UsageSnapshot } | { status: 'unsupported' } | { status: 'error'; retryAfter?: string }

export async function fetchAccountUsage($: EngineInterface): Promise<AccountUsageResult> {
  let authorization: SessionAuthorization
  try {
    authorization = await $.session.authorize()
  } catch {
    return { status: 'unsupported' }
  }
  if (authorization?.kind !== 'bearer') return { status: 'unsupported' }
  try {
    const response = await $.http.fetch('https://api.anthropic.com/api/oauth/usage', {
      auth: authorization.handle,
      headers: { 'anthropic-beta': 'oauth-2025-04-20', accept: 'application/json' },
    })
    if (!response.ok) return { status: 'error', retryAfter: response.status === 429 ? response.headers['retry-after'] : undefined }
    const payload: unknown = JSON.parse(response.text)
    if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return { status: 'error' }
    const readings: RateLimitReading[] = []
    for (const kind of ['five_hour', 'seven_day', 'seven_day_opus', 'seven_day_sonnet']) {
      const value: unknown = (payload as Record<string, unknown>)[kind]
      if (!value || typeof value !== 'object' || Array.isArray(value)) continue
      const window = value as Record<string, unknown>
      if (typeof window.utilization !== 'number') continue
      readings.push({
        kind,
        percentUsed: window.utilization,
        resetsAt: typeof window.resets_at === 'string' ? window.resets_at : undefined,
      })
    }
    const snapshot = normalizeUsage(readings, await $.clock.now())
    return snapshot.status === 'ready' ? { status: 'ready', snapshot } : { status: 'error' }
  } catch {
    return { status: 'error' }
  }
}

export function retryAfterTime(value: string | undefined, now: number): number {
  if (value === undefined) return 0
  const trimmed = value.trim()
  if (/^\d+(?:\.\d+)?$/.test(trimmed)) {
    const delay = Number(trimmed) * 1000
    return Number.isFinite(now + delay) ? now + delay : 0
  }
  const date = Date.parse(trimmed)
  return Number.isFinite(date) && date > now ? date : 0
}
