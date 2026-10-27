export type PromotionCampaign = "claude_mod"

export type PromotionAction =
  | { type: "screen"; label: string; target: "announcement" | "setup" }
  | { type: "external"; label: string; url: string }
  | { type: "app"; label: string; target: "settings"; pane: "usage" }

export type PromotionMediaSource = {
  kind: "image" | "video"
  src: string
  poster?: string
}

export type PromotionMedia = PromotionMediaSource & {
  alt: string
  light?: PromotionMediaSource
}

export type PromotionFeature = {
  icon: "gauge" | "clock"
  title: string
  description: string
}

export type PromotionStep = {
  title: string
  command?: string
  description?: string
}

export type PromotionAnnouncement = {
  eyebrow?: string
  title: string
  description: string
  media?: PromotionMedia
  features?: PromotionFeature[]
  primary_action?: PromotionAction
  secondary_action?: PromotionAction
}

export type PromotionSetup = {
  title: string
  description: string
  steps: PromotionStep[]
  secondary_action?: PromotionAction
}

export type PromotionDefinition = {
  schema_version: 1
  id: string
  revision: number
  title: string
  development_only?: boolean
  analytics?: { campaign: PromotionCampaign }
  screens: {
    announcement: PromotionAnnouncement
    setup?: PromotionSetup
  }
}
