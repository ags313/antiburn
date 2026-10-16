export type UsageState = {
  snapshot: {
    status: 'ready' | 'empty' | 'error'
    windows: { kind: string; label: string; percent: number | null; resetsAt: string | null; startsOnMessage?: boolean }[]
    capturedAt: number
  }
  revealed: string | null
  activated: string | null
  hovered: { target: string; requestId: string; binding: string } | null
}

declare module 'claude-code' {
  interface PluginState {
    'antiburn-mod': { usage: UsageState }
  }
}
