import { parseDocument } from "yaml"
import { isAllowedPromotionUrl } from "./links"

import type {
  PromotionAction,
  PromotionDefinition,
  PromotionFeature,
  PromotionMedia,
  PromotionSetup,
  PromotionStep,
} from "./types"

function fail(location: string, message: string): never {
  throw new Error(`Invalid promotion ${location}: ${message}`)
}

function object(
  value: unknown,
  location: string,
  allowed: string[],
  required: string[] = [],
): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return fail(location, "expected a mapping")
  }

  const record = value as Record<string, unknown>
  for (const key of Object.keys(record)) {
    if (!allowed.includes(key)) fail(location, `unknown key "${key}"`)
  }
  for (const key of required) {
    if (!(key in record)) fail(location, `missing required key "${key}"`)
  }
  return record
}

function text(value: unknown, location: string): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    return fail(location, "expected non-empty text")
  }
  return value
}

function action(value: unknown, location: string): PromotionAction {
  const common = ["type", "label"]
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return fail(location, "expected an action mapping")
  }
  const type = (value as Record<string, unknown>).type
  if (type === "screen") {
    const entry = object(value, location, [...common, "target"], [...common, "target"])
    const target = text(entry.target, `${location}.target`)
    if (target !== "announcement" && target !== "setup")
      fail(`${location}.target`, "unsupported screen")
    return { type, label: text(entry.label, `${location}.label`), target }
  }
  if (type === "external") {
    const entry = object(value, location, [...common, "url"], [...common, "url"])
    const url = text(entry.url, `${location}.url`)
    let parsed: URL
    try {
      parsed = new URL(url)
    } catch {
      return fail(`${location}.url`, "expected an absolute HTTPS URL")
    }
    if (!isAllowedPromotionUrl(parsed)) {
      fail(
        `${location}.url`,
        "only credential-free HTTPS links within the antiburn GitHub repository are supported",
      )
    }
    return { type, label: text(entry.label, `${location}.label`), url }
  }
  if (type === "app") {
    const entry = object(
      value,
      location,
      [...common, "target", "pane"],
      [...common, "target", "pane"],
    )
    if (entry.target !== "settings" || entry.pane !== "usage")
      fail(location, "unsupported app destination")
    return {
      type,
      label: text(entry.label, `${location}.label`),
      target: "settings",
      pane: "usage",
    }
  }
  return fail(`${location}.type`, "expected screen, external, or app")
}

function mediaSource(value: unknown, kind: "image" | "video", location: string): string {
  const source = text(value, location)
  if (
    !source.startsWith("./") ||
    source.includes("\\") ||
    source.includes("?") ||
    source.includes("#") ||
    /^[a-z][a-z\d+.-]*:/i.test(source) ||
    source.split("/").some((segment) => segment === ".." || segment === "")
  ) {
    return fail(location, "expected a relative local asset path")
  }
  const extension = source.slice(source.lastIndexOf(".")).toLowerCase()
  const supported =
    kind === "image"
      ? [".png", ".jpg", ".jpeg", ".webp", ".gif", ".svg"]
      : [".mp4", ".webm", ".mov"]
  if (!supported.includes(extension))
    fail(location, `unsupported ${kind} asset extension "${extension}"`)
  return source
}

function parseMedia(value: unknown, location: string): PromotionMedia {
  const entry = object(
    value,
    location,
    ["kind", "src", "poster", "alt", "light"],
    ["kind", "src", "alt"],
  )
  if (entry.kind !== "image" && entry.kind !== "video")
    fail(`${location}.kind`, "expected image or video")
  const kind = entry.kind
  const src = mediaSource(entry.src, kind, `${location}.src`)
  const poster =
    entry.poster === undefined
      ? undefined
      : mediaSource(entry.poster, "image", `${location}.poster`)
  if (kind === "video" && poster === undefined)
    fail(`${location}.poster`, "video requires a static poster")
  return {
    kind,
    src,
    ...(poster ? { poster } : {}),
    alt: text(entry.alt, `${location}.alt`),
    ...(entry.light === undefined
      ? {}
      : { light: parseMediaVariant(entry.light, `${location}.light`) }),
  }
}

function parseMediaVariant(value: unknown, location: string) {
  const entry = object(value, location, ["kind", "src", "poster"], ["kind", "src"])
  const media = parseMedia({ ...entry, alt: "Theme variant" }, location)
  return { kind: media.kind, src: media.src, ...(media.poster ? { poster: media.poster } : {}) }
}

function parseFeature(value: unknown, location: string): PromotionFeature {
  const entry = object(
    value,
    location,
    ["icon", "title", "description"],
    ["icon", "title", "description"],
  )
  if (entry.icon !== "gauge" && entry.icon !== "clock")
    fail(`${location}.icon`, "unsupported icon")
  return {
    icon: entry.icon,
    title: text(entry.title, `${location}.title`),
    description: text(entry.description, `${location}.description`),
  }
}

function parseStep(value: unknown, location: string): PromotionStep {
  const entry = object(value, location, ["title", "command", "description"], ["title"])
  if (entry.command !== undefined && entry.description !== undefined) {
    fail(location, "provide either command or description, not both")
  }
  return {
    title: text(entry.title, `${location}.title`),
    ...(entry.command === undefined
      ? {}
      : { command: text(entry.command, `${location}.command`) }),
    ...(entry.description === undefined
      ? {}
      : { description: text(entry.description, `${location}.description`) }),
  }
}

function optionalAction(
  entry: Record<string, unknown>,
  key: string,
  location: string,
): PromotionAction | undefined {
  return entry[key] === undefined ? undefined : action(entry[key], `${location}.${key}`)
}

export function validatePromotionDefinition(value: unknown): PromotionDefinition {
  const root = object(
    value,
    "root",
    ["schema_version", "id", "revision", "title", "development_only", "analytics", "screens"],
    ["schema_version", "id", "revision", "title", "screens"],
  )
  if (root.schema_version !== 1) fail("schema_version", "only version 1 is supported")
  if (typeof root.id !== "string" || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(root.id)) {
    fail("id", "expected a lowercase kebab-case identifier")
  }
  if (!Number.isSafeInteger(root.revision) || (root.revision as number) < 1) {
    fail("revision", "expected a positive integer")
  }
  if (root.development_only !== undefined && typeof root.development_only !== "boolean") {
    fail("development_only", "expected a boolean")
  }

  let analytics: PromotionDefinition["analytics"]
  if (root.analytics !== undefined) {
    const entry = object(root.analytics, "analytics", ["campaign"], ["campaign"])
    if (entry.campaign !== "claude_mod")
      fail("analytics.campaign", "unsupported registered campaign")
    analytics = { campaign: entry.campaign }
  }

  const screens = object(root.screens, "screens", ["announcement", "setup"], ["announcement"])
  const announcementRaw = object(
    screens.announcement,
    "screens.announcement",
    [
      "eyebrow",
      "title",
      "description",
      "media",
      "features",
      "primary_action",
      "secondary_action",
    ],
    ["title", "description"],
  )
  const announcement = {
    ...(announcementRaw.eyebrow === undefined
      ? {}
      : { eyebrow: text(announcementRaw.eyebrow, "screens.announcement.eyebrow") }),
    title: text(announcementRaw.title, "screens.announcement.title"),
    description: text(announcementRaw.description, "screens.announcement.description"),
    ...(announcementRaw.media === undefined
      ? {}
      : { media: parseMedia(announcementRaw.media, "screens.announcement.media") }),
    ...(announcementRaw.features === undefined
      ? {}
      : {
          features: (() => {
            if (!Array.isArray(announcementRaw.features))
              fail("screens.announcement.features", "expected a list")
            return announcementRaw.features.map((feature, index) =>
              parseFeature(feature, `screens.announcement.features[${index}]`),
            )
          })(),
        }),
    ...(optionalAction(announcementRaw, "primary_action", "screens.announcement")
      ? {
          primary_action: optionalAction(
            announcementRaw,
            "primary_action",
            "screens.announcement",
          )!,
        }
      : {}),
    ...(optionalAction(announcementRaw, "secondary_action", "screens.announcement")
      ? {
          secondary_action: optionalAction(
            announcementRaw,
            "secondary_action",
            "screens.announcement",
          )!,
        }
      : {}),
  }

  let setup: PromotionSetup | undefined
  if (screens.setup !== undefined) {
    const setupRaw = object(
      screens.setup,
      "screens.setup",
      ["title", "description", "steps", "secondary_action"],
      ["title", "description", "steps"],
    )
    if (!Array.isArray(setupRaw.steps)) fail("screens.setup.steps", "expected a list")
    setup = {
      title: text(setupRaw.title, "screens.setup.title"),
      description: text(setupRaw.description, "screens.setup.description"),
      steps: setupRaw.steps.map((step, index) =>
        parseStep(step, `screens.setup.steps[${index}]`),
      ),
      ...(optionalAction(setupRaw, "secondary_action", "screens.setup")
        ? { secondary_action: optionalAction(setupRaw, "secondary_action", "screens.setup")! }
        : {}),
    }
  }

  for (const [screenName, screen] of Object.entries({
    announcement,
    ...(setup ? { setup } : {}),
  })) {
    for (const [actionName, screenAction] of Object.entries(screen)) {
      if (
        (actionName === "primary_action" || actionName === "secondary_action") &&
        screenAction
      ) {
        const parsedAction = screenAction as PromotionAction
        if (parsedAction.type === "screen" && parsedAction.target === "setup" && !setup) {
          fail(`screens.${screenName}.${actionName}.target`, "setup screen is not defined")
        }
      }
    }
  }

  return {
    schema_version: 1,
    id: root.id,
    revision: root.revision as number,
    title: text(root.title, "title"),
    ...(root.development_only === undefined ? {} : { development_only: root.development_only }),
    ...(analytics ? { analytics } : {}),
    screens: { announcement, ...(setup ? { setup } : {}) },
  }
}

export function parsePromotionYaml(source: string, location: string): PromotionDefinition {
  const document = parseDocument(source, { uniqueKeys: true, prettyErrors: true })
  if (document.errors.length > 0)
    throw new Error(`${location}: ${document.errors.map((error) => error.message).join("\n")}`)
  return validatePromotionDefinition(document.toJS())
}
