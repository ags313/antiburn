import { PushButton } from "../../components/ui/PushButton"
import {
  contextLimitError,
  type Connection,
  type ModelCapabilities,
} from "../../lib/smartCheckProviders"
import { SettingsDisclosure, SettingsField } from "./SettingsSearchRows"

const LIMIT_FIELDS = [
  ["decisionTotalInput", "totalInputTokens"],
  ["decisionStateInput", "stateAndLongestQuestionTokens"],
  ["decisionRuntimeContext", "runtimeContextTokens"],
] as const

const SOURCES = {
  provider_metadata: "provider",
  runtime_metadata: "runtime",
  documented_default: "documented",
  manual: "manual",
  unknown: "unknown",
} as const

export function ProviderModelLimits({
  connection,
  capabilities,
  control,
  targetRevision,
  disabled,
  refreshing,
  onChange,
  onRefresh,
}: {
  connection: Connection
  capabilities: ModelCapabilities | null
  control: string | null | undefined
  targetRevision: number | undefined
  disabled: boolean
  refreshing: boolean
  onChange: (limits: Connection["contextOverride"]) => void
  onRefresh: () => void
}) {
  const targetOpen =
    control === "decisionModelLimits" || LIMIT_FIELDS.some(([id]) => id === control)
  const error = contextLimitError(connection)
  return (
    <SettingsDisclosure
      key={`${control}:${targetRevision}`}
      searchId="decisionModelLimits"
      className="px-3 py-1"
      defaultOpen={targetOpen}
    >
      <PushButton disabled={disabled} onClick={onRefresh}>
        {refreshing ? "Refreshing…" : "Refresh model limits"}
      </PushButton>
      {capabilities && (
        <dl className="mt-3 grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1 type-footnote">
          <dt>Choice decisions</dt>
          <dd>{capabilities.output.choice ? "Supported" : "Unavailable"}</dd>
          <dt>Label probabilities</dt>
          <dd>{capabilities.output.choice?.probabilities ? "Available" : "Unavailable"}</dd>
          {(
            [
              ["Total input tokens", capabilities.total_input_tokens],
              [
                "State and longest question tokens",
                capabilities.state_and_longest_question_tokens,
              ],
              ["Loaded context tokens", capabilities.runtime_context_tokens],
              ["Request bytes", capabilities.request_body_bytes],
              ["Questions per request", capabilities.questions_per_request],
            ] as const
          ).map(([label, limit]) => (
            <div key={label} className="contents">
              <dt>{label}</dt>
              <dd>
                {limit.value?.toLocaleString() ?? "Unknown"} ({SOURCES[limit.source]})
              </dd>
            </div>
          ))}
        </dl>
      )}
      <p className="mt-3 type-footnote text-label-secondary">
        Leave overrides empty to use detected limits. Overrides do not add model features.
      </p>
      <div className="mt-3 space-y-3">
        {LIMIT_FIELDS.map(([id, field]) => (
          <SettingsField key={id} searchId={id}>
            <input
              type="number"
              min={1}
              max={65536}
              step={1}
              disabled={disabled}
              className="mt-1 min-h-[var(--control-height-regular)] w-full rounded-control border border-separator bg-input-fill px-2 type-body text-label"
              value={connection.contextOverride?.[field] ?? ""}
              placeholder="Automatic"
              onChange={(event) => {
                const limits = {
                  totalInputTokens: null,
                  stateAndLongestQuestionTokens: null,
                  runtimeContextTokens: null,
                  ...connection.contextOverride,
                  [field]: event.target.value === "" ? null : Number(event.target.value),
                }
                onChange(Object.values(limits).every((value) => value === null) ? null : limits)
              }}
            />
          </SettingsField>
        ))}
      </div>
      {error && (
        <p role="alert" className="mt-2 type-footnote text-system-red-text">
          {error}
        </p>
      )}
    </SettingsDisclosure>
  )
}
