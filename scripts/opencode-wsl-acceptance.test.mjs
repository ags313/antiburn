import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { parse } from "yaml";

import {
  planOutputs,
  resolveStage,
  routeChanges,
} from "./classify-ci-changes.mjs";

const workflow = parse(
  readFileSync(new URL("../.github/workflows/ci.yml", import.meta.url), "utf8"),
);
const script = readFileSync(
  new URL("./run-opencode-wsl-acceptance.ps1", import.meta.url),
  "utf8",
);

test("manual dispatch selects native platforms and the WSL engine workspace", () => {
  const output = planOutputs({
    stage: resolveStage({ eventName: "workflow_dispatch" }),
    changes: routeChanges([]),
  });
  assert.deepEqual(
    JSON.parse(output.rust_platforms).map((platform) => platform.name),
    ["linux", "linux-arm64", "windows", "windows-arm64", "macos"],
  );
  assert.ok(output.engine_workspaces.split(",").includes("engine"));
  assert.equal(output.run_engine, "true");
});

test("selected Windows x64 engine jobs must run real WSL acceptance", () => {
  const step = workflow.jobs.engine.steps.find(
    (step) => step.run === "./scripts/run-opencode-wsl-acceptance.ps1",
  );
  assert.ok(step);
  assert.equal(
    step.if,
    "matrix.name == 'windows' && contains(format(',{0},', needs.classify.outputs.engine_workspaces), ',engine,')",
  );
  assert.equal(step.shell, "pwsh");
  assert.equal(step["timeout-minutes"], 15);
  assert.notEqual(step["continue-on-error"], true);
  assert.ok(workflow.jobs["ci-required"].needs.includes("engine"));
});

test("WSL provisioning and contract changes select their real engine acceptance", () => {
  for (const file of [
    "scripts/run-opencode-wsl-acceptance.ps1",
    "scripts/opencode-wsl-acceptance.test.mjs",
  ]) {
    const changes = routeChanges([file]);
    const output = planOutputs({ stage: "ready", changes });
    assert.ok(changes.workspaces.includes("engine"));
    assert.equal(output.run_engine, "true");
    assert.equal(output.run_ci_scripts, "true");
    assert.ok(
      JSON.parse(output.rust_platforms).some(
        (platform) => platform.name === "windows",
      ),
    );
  }
});

test("WSL setup runs only the named ignored test and removes its own distro", () => {
  assert.match(script, /RUNNER_ENVIRONMENT -ne 'github-hosted'/);
  assert.match(script, /'AntiburnAcceptance-' \+ \[guid\]::NewGuid\(\)/);
  assert.match(script, /wsl\.exe --import \$name .* --version 2/);
  assert.match(
    script,
    /--test opencode_wsl_acceptance --run-ignored ignored-only/,
  );
  assert.match(
    script,
    /test\(production_bridge_reads_registered_wsl_service_without_starting_opencode\)/,
  );
  assert.match(script, /finally \{[\s\S]*wsl\.exe --unregister \$name/);
  assert.match(script, /wsl\.exe --terminate \$name/);
  assert.match(script, /wsl\.exe --distribution \$name --exec id -un/);
  assert.match(script, /\$defaultUser -ne 'antiburn'/);
  assert.doesNotMatch(script, /--shutdown|--set-default|--run-ignored all/);
});
