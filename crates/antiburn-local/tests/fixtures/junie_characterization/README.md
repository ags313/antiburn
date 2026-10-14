# Junie Events Fixtures

`events.jsonl` is a synthetic JetBrains Junie session journal. It copies the
record shapes observed in `~/.junie/sessions/session-*/events.jsonl` on macOS
and Linux installs in October 2026. All values are synthetic.

The file holds inert lifecycle records, inert agent UI blocks, three main-agent
`LlmResponseMetadataEvent` usage records, and one `CustomAgent` usage record.
The cost-trajectory snapshot repeats the session totals and must not add usage.
