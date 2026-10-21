/**
 * Agent display registry.
 *
 * Maps each engine slug to its display name, icon slot, session surface, and
 * usable session analysis support.
 *
 * The registry holds an icon *name*, never artwork. Rendering an icon is the
 * caller's job: components in this app take a `renderAgentIcon` slot and are
 * perfectly happy when nothing fills it.
 */

/** Where a session was discovered from. */
export type AgentSurface = "cli" | "ide_desktop" | "unknown"

/** A call site can mute vendor colour without changing the vendor artwork. */
export type AgentIconAppearance = "default" | "neutral"

/** Style for one agent in Overview charts. */
export interface AgentStyle {
  fill: string
  swatch: string
}

/** Everything the presentation layer knows about one agent. */
interface AgentInfo {
  displayName: string
  /** Icon slot name. A name, not an asset — see the module comment. */
  icon: string
  /**
   * Slug-only fallback for the session surface. Bi-modal agents (both a CLI
   * and an editor) return `unknown`; only a classified session can say which.
   */
  defaultSurface: AgentSurface
  /**
   * Whether the engine has a usable session parser for this agent.
   * This does not describe discovery or Burn Check support. Passive source
   * registration does not enable analysis.
   */
  supportsAnalysis: boolean
  /** Chart fill class for Overview plots. */
  fill: string
  /** Swatch background class for Overview legends. */
  swatch: string
}

/** Fallback icon slot name for a slug the registry does not know. */
export const GENERIC_AGENT_ICON = "generic-agent"

/** Fallback style for an agent without an Overview colour. */
export const OTHER_AGENT_STYLE: AgentStyle = {
  fill: "fill-agent-other",
  swatch: "bg-agent-other",
}

const AGENTS: Record<string, AgentInfo> = {
  "claude-code": {
    displayName: "Claude Code",
    icon: "claude",
    defaultSurface: "unknown",
    supportsAnalysis: true,
    fill: "fill-agent-claude-code",
    swatch: "bg-agent-claude-code",
  },
  codex: {
    displayName: "Codex",
    icon: "codex",
    defaultSurface: "unknown",
    supportsAnalysis: true,
    fill: "fill-agent-codex",
    swatch: "bg-agent-codex",
  },
  cursor: {
    displayName: "Cursor",
    icon: "cursor",
    defaultSurface: "unknown",
    supportsAnalysis: true,
    fill: "fill-agent-cursor",
    swatch: "bg-agent-cursor",
  },
  copilot: {
    displayName: "GitHub Copilot",
    icon: "copilot",
    defaultSurface: "unknown",
    supportsAnalysis: false,
    fill: "fill-agent-copilot",
    swatch: "bg-agent-copilot",
  },
  cline: {
    displayName: "Cline",
    icon: "cline",
    defaultSurface: "unknown",
    supportsAnalysis: true,
    fill: "fill-agent-cline",
    swatch: "bg-agent-cline",
  },
  opencode: {
    displayName: "OpenCode",
    icon: "opencode",
    defaultSurface: "cli",
    supportsAnalysis: true,
    fill: "fill-agent-opencode",
    swatch: "bg-agent-opencode",
  },
  kiro: {
    displayName: "Kiro",
    icon: "kiro",
    defaultSurface: "unknown",
    supportsAnalysis: true,
    fill: "fill-agent-kiro",
    swatch: "bg-agent-kiro",
  },
  "amp-code": {
    displayName: "Amp",
    icon: "amp",
    defaultSurface: "cli",
    supportsAnalysis: false,
    fill: "fill-agent-amp-code",
    swatch: "bg-agent-amp-code",
  },
  antigravity: {
    displayName: "Antigravity",
    icon: "antigravity",
    defaultSurface: "unknown",
    supportsAnalysis: true,
    fill: "fill-agent-antigravity",
    swatch: "bg-agent-antigravity",
  },
  windsurf: {
    displayName: "Devin",
    icon: "windsurf",
    defaultSurface: "ide_desktop",
    supportsAnalysis: true,
    fill: "fill-agent-windsurf",
    swatch: "bg-agent-windsurf",
  },
  pi: {
    displayName: "Pi",
    icon: "pi",
    defaultSurface: "cli",
    supportsAnalysis: true,
    fill: "fill-agent-pi",
    swatch: "bg-agent-pi",
  },
  omp: {
    displayName: "Oh My Pi",
    icon: "omp",
    defaultSurface: "cli",
    supportsAnalysis: true,
    fill: "fill-agent-omp",
    swatch: "bg-agent-omp",
  },
  junie: {
    displayName: "Junie",
    icon: "junie",
    defaultSurface: "unknown",
    supportsAnalysis: true,
    fill: "fill-agent-junie",
    swatch: "bg-agent-junie",
  },
}

/** Every agent slug the registry knows, in declaration order. */
export const AGENT_SLUGS: readonly string[] = Object.keys(AGENTS)

/**
 * A user-friendly display name for a slug.
 *
 * An unknown slug is title-cased rather than dropped, so a newly released
 * agent still renders as *something* before the registry catches up.
 */
export function agentDisplayName(slug: string): string {
  return (
    AGENTS[slug]?.displayName ??
    slug.replace(/-/g, " ").replace(/\b\w/g, (c) => c.toUpperCase())
  )
}

/** The icon slot name for a slug; {@link GENERIC_AGENT_ICON} when unknown. */
export function agentIconName(slug: string): string {
  return AGENTS[slug]?.icon ?? GENERIC_AGENT_ICON
}

/**
 * Slug-only fallback for the session surface. Use when a session carries no
 * classified surface yet. Bi-modal agents return `unknown` here.
 */
export function defaultAgentSurface(slug: string): AgentSurface {
  return AGENTS[slug]?.defaultSurface ?? "unknown"
}

/**
 * Whether the engine has a usable session parser for this agent.
 */
export function agentSupportsAnalysis(slug: string): boolean {
  return AGENTS[slug]?.supportsAnalysis ?? false
}

/**
 * The provider that bills an agent's usage, for an agent this app routes to
 * one fixed provider. Mirrors the `Route::Fixed` arms of the Rust
 * `route_for_agent` in `provider_usage::providers`.
 *
 * An agent whose provider depends on the model in play (`cline`, `opencode`,
 * `pi`), or a slug this registry does not know, has no single answer here.
 * It returns `null`. A caller cannot assert a live limit is absent for it.
 */
const AGENT_PROVIDER: Readonly<Record<string, string>> = {
  "claude-code": "anthropic",
  codex: "openai",
  copilot: "github",
  cursor: "cursor",
  antigravity: "google",
  windsurf: "windsurf",
  "amp-code": "amp",
  kiro: "kiro",
  junie: "jetbrains",
}

export function agentProvider(slug: string): string | null {
  return AGENT_PROVIDER[slug] ?? null
}

/** Use the same agent filter label in the sidebar and search. */
export function agentSessionFilterLabel(agent: string): string {
  return `${agentDisplayName(agent)} Sessions`
}

/** The chart colour classes for one agent slug. */
export function agentStyle(slug: string): AgentStyle {
  const agent = AGENTS[slug]
  if (!agent) {
    return OTHER_AGENT_STYLE
  }
  return { fill: agent.fill, swatch: agent.swatch }
}
