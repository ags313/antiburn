import { useState, useSyncExternalStore } from "react"

import { Card } from "../../components/ui/Card"
import { Disclosure } from "../../components/ui/Disclosure"
import { PushButton } from "../../components/ui/PushButton"
import {
  connectionDraftChanged,
  contextLimitError,
  defaultConnection,
  SMART_CHECK_PROVIDERS,
  type Connection,
  type SmartCheckProvider,
} from "../../lib/smartCheckProviders"
import type { CheckAvailability } from "../../lib/checkAvailability"
import { settingsControlLabel, type SettingsControlId } from "../../lib/settingsSearchTargets"
import { detectPlatform } from "../../lib/platform"
import { ProviderSettingsSession } from "./ProviderSettingsSession"
import { ProviderModelLimits } from "./ProviderModelLimits"
import { SettingsField, SettingsRow } from "./SettingsSearchRows"

const providerInputClass =
  "min-h-[var(--control-height-regular)] w-full rounded-control border border-separator bg-input-fill px-2 type-body text-label"

export function CheckProviderSettings({
  control,
  targetRevision,
  legacyKeySaved = false,
  usage,
}: {
  control?: string | null | undefined
  targetRevision?: number | undefined
  legacyKeySaved?: boolean
  usage?: CheckAvailability["usage"]
}) {
  const [session] = useState(() => new ProviderSettingsSession())
  const state = useSyncExternalStore(
    session.subscribe,
    session.getSnapshot,
    session.getSnapshot,
  )
  const current = state.drafts[state.selectedId]
  const connection = current?.draft.connection ?? defaultConnection("jev")
  const savedConnection = state.saved?.profiles[state.selectedId]
  const active = state.saved?.profiles[state.saved.activeId]
  const hasSavedCredential =
    savedConnection?.credential?.kind === "legacy_type_safe"
      ? legacyKeySaved
      : Boolean(savedConnection?.credential)
  const credential = current?.draft.credential ?? ""
  const unavailable = state.loading || !state.saved || state.busy
  const changed = current ? connectionDraftChanged(current.draft, savedConnection) : false
  const limitError = contextLimitError(connection)
  const needsCredential = connection.provider === "jev" || connection.provider === "cloudflare"
  const disabled =
    unavailable ||
    !connection.model.trim() ||
    (connection.endpoint.kind !== "provider_default" && !connection.endpoint.value.trim()) ||
    (needsCredential && !credential.trim() && !hasSavedCredential) ||
    Boolean(limitError)

  function edit(update: Partial<Connection>) {
    session.edit({ ...connection, ...update }, credential || null)
  }

  function textField(
    searchId: SettingsControlId,
    value: string,
    onChange: (value: string) => void,
    secret = false,
  ) {
    return (
      <SettingsField searchId={searchId}>
        <input
          type={secret ? "password" : "text"}
          autoComplete="off"
          spellCheck={false}
          className={`mt-1 ${providerInputClass}`}
          value={value}
          disabled={unavailable}
          placeholder={secret && hasSavedCredential ? "Saved; leave empty to keep" : undefined}
          onChange={(event) => onChange(event.target.value)}
        />
      </SettingsField>
    )
  }

  const endpointField = (searchId: SettingsControlId) =>
    textField(
      searchId,
      connection.endpoint.kind === "provider_default" ? "" : connection.endpoint.value,
      (value) => {
        if (connection.endpoint.kind !== "provider_default")
          edit({ endpoint: { ...connection.endpoint, value } })
      },
    )
  const modelField = (searchId: SettingsControlId) =>
    textField(searchId, connection.model, (model) => edit({ model }))
  const keyField = (searchId: SettingsControlId) =>
    textField(searchId, credential, (value) => session.edit(connection, value || null), true)

  return (
    <div
      className="space-y-2"
      ref={() => {
        session.revealTarget(control, targetRevision)
      }}
    >
      <p className="wrap-anywhere px-1 type-footnote text-label-secondary">
        {active
          ? `Active: ${SMART_CHECK_PROVIDERS[active.provider].label} · ${active.model}`
          : "No active connection loaded."}
      </p>
      <Card>
        <SettingsRow
          searchId="decisionProvider"
          trailing={
            <select
              aria-label="Provider"
              className={`${providerInputClass} max-w-64`}
              disabled={unavailable}
              value={connection.provider}
              onChange={(event) =>
                session.selectProvider(event.target.value as SmartCheckProvider)
              }
            >
              {Object.entries(SMART_CHECK_PROVIDERS).map(([id, provider]) => (
                <option key={id} value={id}>
                  {provider.label}
                </option>
              ))}
            </select>
          }
        />
        <SettingsRow
          searchId="decisionConnection"
          trailing={
            <select
              aria-label="Saved connection"
              className={`${providerInputClass} max-w-64`}
              disabled={unavailable}
              value={savedConnection ? state.selectedId : ""}
              onChange={(event) => session.select(event.target.value)}
            >
              {!savedConnection && <option value="">New connection</option>}
              {Object.entries(state.saved?.profiles ?? {}).map(([id, profile]) => (
                <option key={id} value={id}>
                  {id} · {profile.model}
                  {state.saved?.activeId === id ? " (active)" : ""}
                </option>
              ))}
            </select>
          }
        />
        <div className="space-y-3 p-4">
          {connection.provider === "jev" && keyField("typeSafeApiKey")}
          {connection.provider === "ollama" && (
            <>
              {endpointField("ollamaUrl")}
              {modelField("ollamaModel")}
              {keyField("ollamaApiKey")}
            </>
          )}
          {connection.provider === "cloudflare" && (
            <>
              {endpointField("cloudflareAccountId")}
              {keyField("cloudflareApiToken")}
              <SettingsField searchId="cloudflareModel">
                <select
                  aria-label={settingsControlLabel("cloudflareModel", detectPlatform())}
                  className={`mt-1 ${providerInputClass}`}
                  value={connection.model}
                  disabled={unavailable}
                  onChange={(event) => edit({ model: event.target.value })}
                >
                  <option value="clef">clef</option>
                  <option value="clef-flash">clef-flash</option>
                </select>
              </SettingsField>
            </>
          )}
          {connection.provider === "custom" && (
            <>
              {endpointField("customUrl")}
              {modelField("customModel")}
              {keyField("customApiKey")}
              <SettingsField searchId="customResponseMode">
                <select
                  aria-label={settingsControlLabel("customResponseMode", detectPlatform())}
                  className={`mt-1 ${providerInputClass}`}
                  value={connection.responseMode}
                  disabled={unavailable}
                  onChange={(event) =>
                    edit({ responseMode: event.target.value as Connection["responseMode"] })
                  }
                >
                  <option value="direct">Direct System One</option>
                  <option value="cloudflare_envelope">Cloudflare envelope</option>
                </select>
              </SettingsField>
            </>
          )}
        </div>
        <ProviderModelLimits
          connection={connection}
          capabilities={current?.capabilities ?? null}
          control={control}
          targetRevision={targetRevision}
          disabled={unavailable}
          refreshing={state.action === "refresh"}
          onChange={(contextOverride) => edit({ contextOverride })}
          onRefresh={() => void session.run("refresh")}
        />
        <div className="p-4">
          <p className="mb-2 type-footnote text-label-secondary">
            {!state.saved
              ? "Load saved connections before editing."
              : changed
                ? "Unsaved draft. Save and use to activate it."
                : state.selectedId !== state.saved?.activeId
                  ? "This saved connection is not active."
                  : "This connection is active."}
          </p>
          <div className="flex flex-wrap gap-2">
            <PushButton disabled={disabled} onClick={() => void session.run("test")}>
              {state.action === "test" ? "Testing…" : "Test connection"}
            </PushButton>
            <PushButton
              variant="primary"
              disabled={disabled}
              onClick={() => void session.run("save")}
            >
              {state.action === "save" ? "Saving…" : "Save and use"}
            </PushButton>
            {savedConnection && state.selectedId !== state.saved?.activeId && (
              <PushButton
                disabled={unavailable || changed}
                onClick={() => void session.run("switch")}
              >
                {state.action === "switch" ? "Switching…" : "Use connection"}
              </PushButton>
            )}
            {hasSavedCredential && (
              <PushButton disabled={unavailable} onClick={() => void session.run("remove")}>
                {state.action === "remove"
                  ? "Removing…"
                  : savedConnection?.credential?.kind === "legacy_type_safe"
                    ? "Remove shared TypeSafe key"
                    : "Remove saved credential"}
              </PushButton>
            )}
          </div>
          <p className="mt-2 type-footnote text-label-secondary">
            Tests use synthetic input and do not activate drafts. Provider charges may apply.
          </p>
          {connection.provider === "custom" && limitError && (
            <p className="mt-2 type-footnote text-label-secondary">
              Set an input bound in Model limits before saving or testing.
            </p>
          )}
          {current?.tested && (
            <p role="status" className="mt-2 type-footnote text-system-green">
              Connection test passed.
            </p>
          )}
          {state.loading && (
            <p role="status" className="mt-2 type-footnote text-label-secondary">
              Loading connections…
            </p>
          )}
          {state.status && (
            <p role="status" className="mt-2 type-footnote text-label-secondary">
              {state.status}
            </p>
          )}
          {state.error && (
            <p role="alert" className="mt-2 type-footnote text-system-red-text">
              {state.error}
            </p>
          )}
          {!state.loading && !state.saved && (
            <PushButton onClick={() => void session.refresh()}>
              Retry loading connections
            </PushButton>
          )}
        </div>
        {usage && (
          <div className="px-3 py-1">
            <Disclosure label="Model usage">
              <p className="type-footnote text-label-secondary">
                Recorded totals on this device. Clear index in Settings → Privacy resets them.
              </p>
              <p className="mt-2 type-footnote text-label-secondary">
                {usage.confirmedCalls > 0 || usage.inputTokens > 0 || usage.outputTokens > 0
                  ? `${usage.inputTokens.toLocaleString()} input tokens · ${usage.outputTokens.toLocaleString()} output tokens · ${usage.confirmedCalls.toLocaleString()} confirmed requests`
                  : "No confirmed model requests yet."}
              </p>
              <p className="mt-1 type-footnote text-label-secondary">
                {usage.estimatedUsd === null
                  ? "Cost estimate unavailable."
                  : `${usage.estimatedUsd} estimated cost. This is not a bill.`}
              </p>
              <p className="mt-1 type-footnote text-label-secondary">
                {usage.cacheHits.toLocaleString()} cached results reused.
              </p>
              {usage.unknownOutcomes > 0 && (
                <p className="mt-1 type-footnote text-label-secondary">
                  {usage.unknownOutcomes.toLocaleString()} request outcomes are unknown and are
                  not included in the estimate.
                </p>
              )}
            </Disclosure>
          </div>
        )}
      </Card>
      <p className="px-1 type-footnote text-label-secondary">
        Checks need a configured active connection; selected session content goes to that
        provider, not full transcripts or private thinking.
      </p>
    </div>
  )
}
