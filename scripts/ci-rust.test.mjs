import assert from "node:assert/strict";
import test from "node:test";

import {
  RUST_WORKSPACES,
  parseArguments,
  parseWorkspaceList,
  planRustChecks,
} from "./ci-rust.mjs";

const ALL_IDS = RUST_WORKSPACES.map((workspace) => workspace.id);

function labels(plan) {
  return plan.steps.map((step) => `${step.workspace}: ${step.step}`);
}

test("runs workspaces in the fixed order, whatever the input order", () => {
  const plan = planRustChecks({
    workspaces: [...ALL_IDS].reverse(),
    platform: "linux",
  });
  const order = [...new Set(plan.steps.map((step) => step.workspace))];
  assert.deepEqual(order, [
    "engine",
    "remote",
    "shell",
    "trace",
    "hud",
    "main-window",
    "anchored-window",
    "nudge",
    "sound",
    "tray-icon",
  ]);
  assert.deepEqual(plan.skipped, []);
});

test("runs fmt, then Clippy, then tests in each workspace", () => {
  const plan = planRustChecks({
    workspaces: "trace,engine",
    fmt: true,
    platform: "linux",
  });
  assert.deepEqual(labels(plan), [
    "engine: fmt",
    "engine: clippy",
    "engine: tests",
    "trace: fmt",
    "trace: clippy",
    "trace: tests",
  ]);
  assert.deepEqual(plan.steps[0].args, ["fmt", "--check"]);
});

test("omits fmt without the flag", () => {
  const plan = planRustChecks({ workspaces: "hud", platform: "linux" });
  assert.ok(plan.steps.every((step) => step.step !== "fmt"));
});

test("uses the per-workspace commands and directories", () => {
  const plan = planRustChecks({
    workspaces: "engine,remote,shell,nudge,tray-icon",
    platform: "linux",
  });
  const byLabel = Object.fromEntries(
    plan.steps.map((step) => [`${step.workspace}: ${step.step}`, step]),
  );
  assert.deepEqual(byLabel["engine: clippy"].args, [
    "clippy",
    "--all-targets",
    "--locked",
    "--",
    "-D",
    "warnings",
  ]);
  assert.deepEqual(byLabel["engine: tests"].args, [
    "nextest",
    "run",
    "--locked",
    "--no-tests=pass",
    "--lib",
    "--tests",
  ]);
  assert.equal(byLabel["engine: tests"].cwd, "crates/antiburn-local");
  assert.deepEqual(byLabel["remote: tests"].args, [
    "nextest",
    "run",
    "--locked",
    "--no-tests=pass",
  ]);
  assert.equal(byLabel["remote: tests"].cwd, "crates/antiburn-remote");
  assert.deepEqual(byLabel["shell: clippy (analytics)"].args, [
    "clippy",
    "--all-targets",
    "--locked",
    "--features",
    "analytics",
    "--",
    "-D",
    "warnings",
  ]);
  assert.deepEqual(byLabel["shell: tests (analytics)"].args, [
    "nextest",
    "run",
    "--locked",
    "--no-tests=pass",
    "--features",
    "analytics",
  ]);
  assert.equal(byLabel["shell: tests (analytics)"].cwd, "apps/desktop/src-tauri");
  assert.deepEqual(byLabel["nudge: tests"].args, [
    "nextest",
    "run",
    "--locked",
    "--no-tests=pass",
  ]);
  assert.equal(
    byLabel["nudge: tests"].cwd,
    "apps/desktop/src-tauri/crates/nudge",
  );
  assert.deepEqual(byLabel["tray-icon: clippy"].args, [
    "clippy",
    "--locked",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ]);
  assert.deepEqual(byLabel["tray-icon: tests"].args, [
    "test",
    "--locked",
    "--lib",
    "--test",
    "macos_highlight_override",
  ]);
  assert.ok(
    plan.steps.every(
      (step) => step.step === "fmt" || step.args.includes("--locked"),
    ),
  );
});

test("sets test env only where the contract needs it", () => {
  const plan = planRustChecks({
    workspaces: "shell,tray-icon,trace",
    platform: "linux",
  });
  const byLabel = Object.fromEntries(
    plan.steps.map((step) => [`${step.workspace}: ${step.step}`, step]),
  );
  assert.deepEqual(byLabel["shell: tests (analytics)"].env, {
    ANTIBURN_ANALYTICS_URL: "http://127.0.0.1:8787",
    ANTIBURN_ANALYTICS_OPERATOR: "Local development",
  });
  assert.deepEqual(byLabel["shell: clippy (analytics)"].env, {});
  assert.deepEqual(byLabel["tray-icon: tests"].env, {
    TRAY_ICON_RUN_NATIVE_TEST: "1",
  });
  assert.deepEqual(byLabel["tray-icon: clippy"].env, {});
  assert.deepEqual(byLabel["trace: tests"].env, {});
  assert.ok(
    plan.steps.every((step) => !("CARGO_TARGET_DIR" in step.env)),
    "commands inherit CARGO_TARGET_DIR from the job",
  );
});

test("adds a shell Clippy pass without analytics only on request", () => {
  const without = planRustChecks({ workspaces: "shell", platform: "linux" });
  assert.deepEqual(labels(without), [
    "shell: clippy (analytics)",
    "shell: tests (analytics)",
  ]);
  const withCheck = planRustChecks({
    workspaces: "shell",
    fmt: true,
    noAnalyticsCheck: true,
    platform: "linux",
  });
  assert.deepEqual(labels(withCheck), [
    "shell: fmt",
    "shell: clippy (analytics)",
    "shell: clippy (no analytics)",
    "shell: tests (analytics)",
  ]);
  assert.deepEqual(withCheck.steps[2].args, [
    "clippy",
    "--all-targets",
    "--locked",
    "--",
    "-D",
    "warnings",
  ]);
  const otherWorkspace = planRustChecks({
    workspaces: "hud",
    noAnalyticsCheck: true,
    platform: "linux",
  });
  assert.deepEqual(labels(otherWorkspace), ["hud: clippy", "hud: tests"]);
});

test("runs the remote helper only on Linux hosts", () => {
  for (const platform of ["darwin", "win32"]) {
    const plan = planRustChecks({ workspaces: "engine,remote", platform });
    assert.deepEqual(
      [...new Set(plan.steps.map((step) => step.workspace))],
      ["engine"],
    );
    assert.equal(plan.skipped.length, 1);
    assert.equal(plan.skipped[0].workspace, "remote");
  }
  const linux = planRustChecks({
    workspaces: "engine,remote",
    platform: "linux",
  });
  assert.deepEqual(
    [...new Set(linux.steps.map((step) => step.workspace))],
    ["engine", "remote"],
  );
});

test("spawns cargo.exe on Windows and cargo elsewhere", () => {
  const windows = planRustChecks({ workspaces: "hud", platform: "win32" });
  assert.ok(windows.steps.every((step) => step.program === "cargo.exe"));
  const macos = planRustChecks({ workspaces: "hud", platform: "darwin" });
  assert.ok(macos.steps.every((step) => step.program === "cargo"));
});

test("rejects unknown workspace ids", () => {
  assert.throws(
    () => planRustChecks({ workspaces: "engine,desktop", platform: "linux" }),
    /Unknown Rust workspace id: desktop/,
  );
  assert.throws(() => parseWorkspaceList("Shell"), /Unknown/);
});

test("plans nothing for an empty list", () => {
  for (const workspaces of ["", " , ", []]) {
    const plan = planRustChecks({ workspaces, fmt: true, platform: "linux" });
    assert.deepEqual(plan, { steps: [], skipped: [] });
  }
});

test("removes duplicate ids", () => {
  const plan = planRustChecks({ workspaces: "hud,hud", platform: "linux" });
  assert.deepEqual(labels(plan), ["hud: clippy", "hud: tests"]);
});

test("parses the command-line flags", () => {
  assert.deepEqual(
    parseArguments([
      "--workspaces",
      "shell,hud",
      "--fmt",
      "--no-analytics-check",
      "--dry-run",
    ]),
    {
      workspaces: "shell,hud",
      fmt: true,
      noAnalyticsCheck: true,
      dryRun: true,
    },
  );
  assert.deepEqual(parseArguments(["--workspaces", ""]), {
    workspaces: "",
    fmt: false,
    noAnalyticsCheck: false,
    dryRun: false,
  });
  assert.equal(parseArguments(["--workspaces=engine"]).workspaces, "engine");
  assert.throws(() => parseArguments([]), /--workspaces is required/);
  assert.throws(() => parseArguments(["--workspaces"]), /needs a value/);
  assert.throws(
    () => parseArguments(["--workspaces", "hud", "--release"]),
    /Unknown argument/,
  );
});
