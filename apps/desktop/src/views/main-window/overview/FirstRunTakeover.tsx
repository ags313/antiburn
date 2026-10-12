import { useState } from "react"

import { cn } from "../../../lib/cn"
import { isMacOS } from "../../../lib/platform"

import { PRIMARY_BUTTON, ProgressStepCard, SKIP_BUTTON } from "./ProgressSteps"
import {
  LIVE_LIMITS_TRANSITION_NAME,
  nextStep,
  previousStep,
  type OverviewProgress,
  type ProgressStepKey,
  progressStepTransitionName,
  showLiveLimits,
  skipLiveLimits,
  stepDone,
} from "./overviewProgressStore"
import { useOverviewProgress } from "./useOverviewProgress"
import { MoreInfoToggle, StepSettingsPanel } from "./stepSettings/StepSettingsDisclosure"

function WelcomeCard() {
  return (
    <div className="flex w-full flex-col gap-(--space-lg) text-center">
      <h2 className="type-large-title text-label">Welcome to antiburn</h2>

      <ul className="mx-auto flex list-disc flex-col gap-(--space-sm) ps-6 text-start type-title-3 font-normal! text-label-secondary">
        <li>Reads and analyses your session logs</li>
        <li>Helps you avoid hitting limits</li>
        <li>100% local, no account needed</li>
      </ul>

      <div className="mt-(--space-lg)">
        <NextButton disabled={false} label="Get Started" onClick={() => void nextStep()} />
      </div>
    </div>
  )
}

function NextButton({
  disabled,
  label = "Next",
  onClick,
}: {
  disabled: boolean
  label?: string
  onClick: () => void
}) {
  return (
    <div className="flex justify-center">
      <button type="button" disabled={disabled} onClick={onClick} className={PRIMARY_BUTTON}>
        {label}
      </button>
    </div>
  )
}

function LiveLimitsCard() {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [infoOpen, setInfoOpen] = useState(false)

  async function handleShow() {
    setBusy(true)
    setError(null)
    try {
      await showLiveLimits()
    } catch {
      setError("Could not start live limits. Try again.")
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex w-full flex-col items-center">
      <div
        style={{ viewTransitionName: LIVE_LIMITS_TRANSITION_NAME }}
        className="flex w-full flex-col items-center gap-(--space-lg)"
      >
        <div className="flex flex-col gap-(--space-sm) text-center">
          <h2 className="type-title-1 text-label">Plan limits</h2>
          <p className="type-title-3 font-normal! text-label-secondary">
            See plan limit usage against your reset times.{" "}
            <MoreInfoToggle
              step="limits"
              open={infoOpen}
              onToggle={() => setInfoOpen(!infoOpen)}
            />
          </p>
          {/* Only macOS keeps these credentials in the Keychain. */}
          {isMacOS() && (
            <p className="mx-auto max-w-lg text-balance type-callout text-label-tertiary">
              macOS may ask for Keychain access, allowing antiburn to read existing coding tool
              credentials.
            </p>
          )}
        </div>

        {infoOpen && <StepSettingsPanel step="limits" />}

        <div className="mt-(--space-lg) grid grid-cols-[1fr_1fr] items-center gap-x-(--space-sm)">
          <button
            type="button"
            disabled={busy}
            onClick={previousStep}
            className="ui-push-button h-full type-title-3 font-normal!"
          >
            Back
          </button>

          <button
            type="button"
            disabled={busy}
            onClick={() => void handleShow()}
            className={PRIMARY_BUTTON}
          >
            {busy ? "Starting…" : "Turn on limits"}
          </button>
        </div>
        <button type="button" disabled={busy} onClick={skipLiveLimits} className={SKIP_BUTTON}>
          Skip until later
        </button>

        {error && (
          <p role="alert" className="type-footnote text-system-red-text">
            {error}
          </p>
        )}
      </div>
    </div>
  )
}

function TakeoverStep({
  step,
  progress,
}: {
  step: ProgressStepKey
  progress: OverviewProgress
}) {
  // Keep the step whose info is open, so the next step starts closed.
  const [infoStep, setInfoStep] = useState<ProgressStepKey | null>(null)
  const infoOpen = infoStep === step
  return (
    <>
      <ProgressStepCard
        step={step}
        progress={progress}
        transitionName={progressStepTransitionName(step)}
        bodyAction={
          step !== "fixes" && (
            <MoreInfoToggle
              step={step}
              open={infoOpen}
              onToggle={() => setInfoStep(infoOpen ? null : step)}
            />
          )
        }
      />

      {step !== "fixes" && infoOpen && <StepSettingsPanel step={step} />}

      <div className="mt-(--space-lg) grid grid-cols-[1fr_1fr] gap-x-(--space-sm)">
        <button
          type="button"
          onClick={previousStep}
          className="ui-push-button h-full type-title-3 font-normal!"
        >
          Back
        </button>
        <NextButton
          disabled={!stepDone(step, progress)}
          label={step === "fixes" ? "Get Started" : "Next"}
          onClick={() => void nextStep()}
        />
      </div>
    </>
  )
}

export function FirstRunTakeover({ className }: { className?: string }) {
  const progress = useOverviewProgress()

  return (
    <fieldset
      disabled={progress.actionPending}
      className={cn(
        "flex flex-1 flex-col items-center justify-center p-(--space-2xl)",
        className,
      )}
    >
      <div className="flex w-full max-w-xl flex-col items-center gap-(--space-lg)">
        {!progress.stepShown ? null : progress.flow === "welcome" ? (
          <WelcomeCard />
        ) : progress.flow === "limits" ? (
          <LiveLimitsCard />
        ) : progress.flow === "agents" ||
          progress.flow === "sessions" ||
          progress.flow === "checks" ||
          progress.flow === "fixes" ? (
          <TakeoverStep step={progress.flow} progress={progress} />
        ) : null}
        {progress.actionError && (
          <p role="alert" className="type-footnote text-system-red-text">
            {progress.actionError}
          </p>
        )}
      </div>
    </fieldset>
  )
}
