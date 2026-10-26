import {
  defaultConnection,
  connectionDraftChanged,
  getProviderSettings,
  refreshProviderLimits,
  removeProviderCredential,
  saveProviderConnection,
  switchProviderConnection,
  testProviderConnection,
  type Connection,
  type ConnectionDraft,
  type ModelCapabilities,
  type ProviderSettings,
  type SmartCheckProvider,
} from "../../lib/smartCheckProviders"
import { removeTypeSafeApiKey } from "../../lib/checkAvailability"
import { confirm } from "@tauri-apps/plugin-dialog"

type DraftState = {
  draft: ConnectionDraft
  tested: boolean
  capabilities: ModelCapabilities | null
}
type Snapshot = {
  loading: boolean
  action: "test" | "save" | "refresh" | "remove" | "switch" | null
  saved: ProviderSettings | null
  selectedId: string
  drafts: Record<string, DraftState>
  busy: boolean
  error: string | null
  status: string | null
}

const safeErrors = new Set([
  "Enter a TypeSafe API key.",
  "Enter a Cloudflare API token.",
  "Credential storage is unavailable.",
  "Could not save the connection credential.",
  "Could not access secure storage. Unlock the system credential store and retry.",
  "Secure storage rejected the credential write. Check app access and retry.",
  "Could not save the API token to secure storage. Check credential storage access and retry.",
  "Could not remove the connection credential.",
  "The connection credential reference is invalid.",
  "Saved Smart Burn Checks provider settings are invalid.",
  "Saved connection settings are invalid.",
  "The saved connection is missing.",
  "Could not save connection settings.",
  "Could not protect the connection update state.",
  "Could not complete the connection update.",
  "Could not protect the credential removal state.",
  "Could not disable checks.",
  "Could not save the credential removal state.",
  "Could not restore the connection credential. Retry the connection update in Settings → Checks.",
  "Could not restore the connection credential. Retry credential removal in Settings → Checks.",
  "Connection credential removal is incomplete. Retry it in Settings → Checks.",
  "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks.",
  "TypeSafe credential removal is incomplete. Retry it in Settings → Checks.",
  "Remove the TypeSafe API key in Settings → Checks.",
  "Could not remove the API key from credential storage.",
  "The selected provider rejected its credential. Update it in Settings → Checks.",
  "The selected provider needs a valid credential.",
  "The selected model provider needs a valid credential.",
  "The selected provider needs a credential. Add one and retry.",
  "Connection settings are invalid.",
  "Enter a valid connection credential.",
  "TypeSafe rejected the test request. Check the API key.",
  "Cloudflare rejected the test request. Check the account, model, and token.",
  "The custom endpoint rejected the test request. Check the URL and response mode.",
  "Could not reach Ollama. Check the server address and try again.",
  "Ollama did not report a usable context size. Set a context override and test again.",
  "Invalid Ollama base URL",
  "Ollama must be version 0.35.0 or newer",
  "The selected Ollama model is not installed",
  "The Ollama model does not support System One decisions",
  "The Ollama model is not loaded; retry after it loads",
  "The Ollama model rejected the request context",
  "Ollama text requests cannot exceed 64 KiB",
  "Ollama rejected the configured authentication",
  "Ollama is unavailable",
  "The Ollama request outcome is unknown",
  "The Ollama request is invalid",
  "Ollama returned an invalid System One response",
  "The Ollama response exceeded the local limit",
  "Could not cancel unfinished Burn Checks. Checks are paused. Retry the connection change.",
  "Could not pause YAML Burn Checks.",
  "Could not enable YAML Burn Checks.",
  "Could not confirm the saved credential. Checks are paused. Retry the connection update in Settings → Checks.",
  "Could not restore the previous credential. Checks are paused. Retry the connection update in Settings → Checks.",
  "Could not restore the previous connection state. Retry the connection update in Settings → Checks.",
  "Could not restore the previous connection. Checks are paused. Retry the connection update in Settings → Checks.",
  "A connection update is incomplete. Retry it in Settings → Checks before removing a credential.",
  "The active model provider credential is missing. Update it in Settings → Checks.",
  "The selected provider does not support this model or runner.",
  "The selected provider did not report usable model limits.",
  "Could not read model limits from the selected provider. Check the connection and try again.",
  "The connection revision limit was reached. Save a new connection.",
])

export function providerErrorMessage(error: unknown, fallback: string): string {
  const message = error instanceof Error ? error.message : error
  return typeof message === "string" && safeErrors.has(message) ? message : fallback
}

export class ProviderSettingsSession {
  private listeners = new Set<() => void>()
  private generation = 0
  private providerSelections = new Map<SmartCheckProvider, string>()
  private targetKey: string | null = null
  private targetProvider: SmartCheckProvider | null = null
  private state: Snapshot = {
    loading: true,
    action: null,
    saved: null,
    selectedId: "jev",
    drafts: {},
    busy: false,
    error: null,
    status: null,
  }
  getSnapshot = () => this.state
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    if (this.listeners.size === 1) {
      void this.refresh()
    }
    return () => {
      this.listeners.delete(listener)
      if (!this.listeners.size) this.generation++
    }
  }
  refresh = async () => {
    if (this.state.busy || (this.state.loading && this.state.saved)) return
    const generation = ++this.generation
    this.publish({ loading: true, error: null })
    try {
      const saved = await getProviderSettings()
      if (generation !== this.generation) return
      if (!saved?.profiles[saved.activeId]) throw new Error("Missing active connection")
      this.load(saved)
    } catch (error) {
      if (generation === this.generation)
        this.publish({
          error: providerErrorMessage(
            error,
            "Could not load saved connections. Retry loading them.",
          ),
        })
    } finally {
      if (generation === this.generation) {
        this.publish({ loading: false })
        this.revealPendingTarget()
      }
    }
  }
  private publish(update: Partial<Snapshot>) {
    this.state = { ...this.state, ...update }
    this.listeners.forEach((listener) => listener())
  }
  private load(saved: ProviderSettings) {
    const drafts = {
      ...this.state.drafts,
      ...Object.fromEntries(
        Object.entries(saved.profiles).map(([id, connection]) => [
          id,
          this.state.drafts[id] &&
          connectionDraftChanged(this.state.drafts[id]!.draft, this.state.saved?.profiles[id])
            ? this.state.drafts[id]!
            : {
                draft: { connection_id: id, connection, credential: null },
                tested: false,
                capabilities: null,
              },
        ]),
      ),
    }
    const selectedId = this.state.saved ? this.state.selectedId : saved.activeId
    this.providerSelections.set(drafts[selectedId]!.draft.connection.provider, selectedId)
    this.publish({ saved, selectedId, drafts, error: null })
  }
  select(id: string) {
    if (this.state.busy || this.state.loading || !this.state.drafts[id]) return
    this.providerSelections.set(this.state.drafts[id]!.draft.connection.provider, id)
    this.publish({ selectedId: id, error: null, status: null })
  }
  revealTarget(control: string | null | undefined, revision: number | undefined) {
    const key = `${control ?? ""}:${revision ?? 0}`
    if (this.targetKey === key) return
    this.targetKey = key
    this.targetProvider =
      control === "typeSafeApiKey"
        ? "jev"
        : control?.startsWith("ollama")
          ? "ollama"
          : control?.startsWith("cloudflare")
            ? "cloudflare"
            : control?.startsWith("custom")
              ? "custom"
              : null
    this.revealPendingTarget()
  }
  private revealPendingTarget() {
    if (this.targetProvider && this.state.saved && !this.state.loading && !this.state.busy) {
      this.selectProvider(this.targetProvider)
      this.targetProvider = null
    }
  }
  selectProvider(provider: SmartCheckProvider) {
    if (this.state.busy || this.state.loading) return
    let id =
      this.providerSelections.get(provider) ??
      Object.entries(this.state.drafts).find(
        ([, value]) => value.draft.connection.provider === provider,
      )?.[0] ??
      provider
    if (this.state.drafts[id]?.draft.connection.provider !== provider) {
      let suffix = 2
      while (this.state.drafts[id]) id = `${provider}-${suffix++}`
    }
    if (!this.state.drafts[id]) {
      this.publish({
        drafts: {
          ...this.state.drafts,
          [id]: {
            draft: {
              connection_id: id,
              connection: defaultConnection(provider),
              credential: null,
            },
            tested: false,
            capabilities: null,
          },
        },
      })
    }
    this.providerSelections.set(provider, id)
    this.publish({ selectedId: id, error: null, status: null })
  }
  edit(connection: Connection, credential: string | null) {
    if (this.state.busy || this.state.loading || !this.state.saved) return
    const id = this.state.selectedId
    this.publish({
      drafts: {
        ...this.state.drafts,
        [id]: {
          draft: { connection_id: id, connection, credential },
          tested: false,
          capabilities: null,
        },
      },
      error: null,
      status: null,
    })
  }
  private acceptSaved(saved: ProviderSettings, id: string, status: string) {
    const connection = saved.profiles[id]
    if (!connection) throw new Error("Missing saved connection")
    this.publish({
      saved,
      drafts: {
        ...this.state.drafts,
        [id]: {
          draft: { connection_id: id, connection, credential: null },
          tested: false,
          capabilities: null,
        },
      },
      status,
    })
  }
  async run(action: "test" | "save" | "refresh" | "remove" | "switch") {
    if (this.state.busy || this.state.loading || !this.state.saved) return
    const id = this.state.selectedId
    let current = this.state.drafts[id]
    if (!current) return
    const savedConnection = this.state.saved.profiles[id]
    const sharedKey = savedConnection?.credential?.kind === "legacy_type_safe"
    if (
      action === "switch" &&
      (!savedConnection ||
        this.state.saved.activeId === id ||
        connectionDraftChanged(current.draft, savedConnection))
    )
      return
    if (action === "remove" && !savedConnection?.credential) return
    if (action === "test") current = { ...current, tested: false }
    if (action === "refresh") current = { ...current, capabilities: null }
    this.publish({
      busy: true,
      action,
      error: null,
      status: null,
      drafts: { ...this.state.drafts, [id]: current },
    })
    try {
      if (action === "test") {
        await testProviderConnection(current.draft)
        this.publish({
          drafts: { ...this.state.drafts, [id]: { ...current, tested: true } },
        })
      } else if (action === "refresh") {
        const capabilities = await refreshProviderLimits(current.draft)
        this.publish({
          drafts: { ...this.state.drafts, [id]: { ...current, capabilities } },
          status: "Model limits refreshed for this draft.",
        })
      } else if (action === "save" || action === "switch") {
        const saved =
          action === "save"
            ? await saveProviderConnection(current.draft)
            : await switchProviderConnection(id)
        this.acceptSaved(
          saved,
          id,
          action === "save" ? "Connection saved and active." : "Connection active.",
        )
      } else {
        if (
          sharedKey &&
          !(await confirm("Remove this key? Other connections that use it will stop working.", {
            title: "Remove shared TypeSafe key",
            kind: "warning",
            okLabel: "Remove key",
            cancelLabel: "Cancel",
          }))
        )
          return
        this.publish({
          drafts: {
            ...this.state.drafts,
            [id]: { ...current, tested: false, capabilities: null },
          },
        })
        if (sharedKey) await removeTypeSafeApiKey()
        else await removeProviderCredential(id)
        const saved = await getProviderSettings()
        const connection = saved.profiles[id]
        if (!connection) throw new Error("The saved connection is missing.")
        this.publish({
          saved,
          drafts: {
            ...this.state.drafts,
            [id]: {
              draft: {
                ...current.draft,
                connection: {
                  ...current.draft.connection,
                  credential: connection.credential,
                  revision: connection.revision,
                },
                credential: null,
              },
              tested: false,
              capabilities: null,
            },
          },
          status: sharedKey
            ? "Shared TypeSafe key removed."
            : "Saved credential removed. The connection remains saved.",
        })
      }
    } catch (error) {
      this.publish({
        error: providerErrorMessage(
          error,
          `Could not ${action === "refresh" ? "refresh model limits" : action === "remove" ? "remove the credential" : action === "switch" ? "select the saved connection" : `${action} the connection`}. Check the settings and try again.`,
        ),
      })
      if (action === "save" || action === "switch" || action === "remove") {
        try {
          const saved = await getProviderSettings()
          if (!saved?.profiles[saved.activeId])
            throw new Error("Missing active connection", { cause: error })
          this.publish({ saved })
        } catch {
          this.publish({
            status: "Could not confirm the active connection. Reopen Settings to refresh it.",
          })
        }
      }
    } finally {
      this.publish({ busy: false, action: null })
      this.revealPendingTarget()
    }
  }
}
