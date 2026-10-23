import { fireEvent, render, screen } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import { DEFAULT_SETTINGS } from "../../lib/ipc"
import { PrivacyPane } from "./PrivacyPane"

it("explains selected provider routing without changing saved settings", () => {
  const update = vi.fn()
  const { container } = render(
    <PrivacyPane settings={DEFAULT_SETTINGS} loaded update={update} info={null} />,
  )

  expect(container).toHaveTextContent(
    "the connection selected under Decision model: Jev, compatible Ollama, Cloudflare, or Custom",
  )
  fireEvent.click(screen.getByRole("button", { name: "When your work can leave this machine" }))
  expect(container).toHaveTextContent(
    "A local or remote endpoint determines where the request goes",
  )
  expect(container).toHaveTextContent(
    "keeps saved connections and credentials but stops new checks",
  )
  expect(container).toHaveTextContent("Requests do not include full transcripts")
  expect(container).not.toHaveTextContent("user-slot text to Jev using your API key")
  expect(update).not.toHaveBeenCalled()
})
