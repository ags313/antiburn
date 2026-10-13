import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyChanges,
  classifyPaths,
  isPureAppReleaseChange,
  isPureEngineReleaseChange,
  listChangedFiles,
  planOutputs,
  resolveStage,
  routeChanges,
  withDependents,
} from "./classify-ci-changes.mjs";

function json(version, extra = {}) {
  return `${JSON.stringify({ name: "antiburn", version, ...extra }, null, 2)}\n`;
}

function cargoToml(version, dependency = "1") {
  return `[package]\nname = "antiburn"\nversion = "${version}"\n\n[dependencies]\nserde = "${dependency}"\n`;
}

function cargoLock(packageName, version, dependencyVersion = "1.0.0") {
  return `version = 4\n\n[[package]]\nname = "${packageName}"\nversion = "${version}"\ndependencies = [\n "serde",\n]\n\n[[package]]\nname = "serde"\nversion = "${dependencyVersion}"\n`;
}

// The desktop lockfile records the application and the remote helper, and a
// release bumps both. A fixture with one of them cannot show that.
function appCargoLock(appVersion, helperVersion, dependencyVersion = "1.0.0") {
  return `version = 4\n\n[[package]]\nname = "antiburn"\nversion = "${appVersion}"\ndependencies = [\n "antiburn-remote",\n "serde",\n]\n\n[[package]]\nname = "antiburn-remote"\nversion = "${helperVersion}"\ndependencies = [\n "serde",\n]\n\n[[package]]\nname = "serde"\nversion = "${dependencyVersion}"\n`;
}

function remoteCargoToml(version, dependency = "1") {
  return `[workspace]\n\n[package]\nname = "antiburn-remote"\nversion = "${version}"\n\n[dependencies]\nserde = "${dependency}"\n`;
}

const APP_RELEASE_FILES = [
  "CHANGELOG.md",
  "apps/desktop/package.json",
  "apps/desktop/src-tauri/Cargo.lock",
  "apps/desktop/src-tauri/Cargo.toml",
  "apps/desktop/src-tauri/tauri.conf.json",
  "crates/antiburn-remote/Cargo.lock",
  "crates/antiburn-remote/Cargo.toml",
];

/** Every manifest of an app release bumped from `from` to `to`, and nothing else. */
function appReleaseValues(from, to) {
  return {
    "base:apps/desktop/package.json": json(from, { private: true }),
    "head:apps/desktop/package.json": json(to, { private: true }),
    "base:apps/desktop/src-tauri/tauri.conf.json": json(from, {
      productName: "antiburn",
    }),
    "head:apps/desktop/src-tauri/tauri.conf.json": json(to, {
      productName: "antiburn",
    }),
    "base:apps/desktop/src-tauri/Cargo.toml": cargoToml(from),
    "head:apps/desktop/src-tauri/Cargo.toml": cargoToml(to),
    "base:apps/desktop/src-tauri/Cargo.lock": appCargoLock(from, from),
    "head:apps/desktop/src-tauri/Cargo.lock": appCargoLock(to, to),
    "base:crates/antiburn-remote/Cargo.toml": remoteCargoToml(from),
    "head:crates/antiburn-remote/Cargo.toml": remoteCargoToml(to),
    "base:crates/antiburn-remote/Cargo.lock": cargoLock(
      "antiburn-remote",
      from,
    ),
    "head:crates/antiburn-remote/Cargo.lock": cargoLock("antiburn-remote", to),
  };
}

function reader(values) {
  return (ref, file) => values[`${ref}:${file}`];
}

test("routes documentation without compiling application code", () => {
  assert.deepEqual(classifyPaths(["README.md", "docs/runbooks/release.md"]), {
    docs_only: true,
    frontend: false,
    engine: false,
    desktop_backend: false,
    full: false,
    release_app: false,
    release_engine: false,
  });
});

test("routes engine changes through both Rust matrices", () => {
  const result = classifyPaths(["crates/antiburn-local/src/lib.rs"]);
  assert.equal(result.engine, true);
  assert.equal(result.desktop_backend, true);
  assert.equal(result.frontend, false);
});

test("routes frontend changes without Rust matrices", () => {
  const result = classifyPaths(["apps/desktop/src/App.tsx"]);
  assert.equal(result.frontend, true);
  assert.equal(result.engine, false);
  assert.equal(result.desktop_backend, false);
});

test("fails closed for workflow and unknown paths", () => {
  for (const file of [
    ".github/workflows/ci.yml",
    "unclassified.config",
    "crates/unknown-crate/src/lib.rs",
    "apps/desktop/src-tauri/crates/new-crate/src/lib.rs",
    "scripts/ci-rust.mjs",
    "scripts/classify-ci-changes.mjs",
    "scripts/package-remote-helper.sh",
  ]) {
    const result = classifyPaths([file]);
    assert.equal(result.full, true, file);
    assert.equal(result.frontend, true, file);
    assert.equal(result.engine, true, file);
    assert.equal(result.desktop_backend, true, file);
  }
});

test("routes dependency manifests to their workspace instead of a full run", () => {
  const lock = classifyPaths(["pnpm-lock.yaml"]);
  assert.equal(lock.full, false);
  assert.equal(lock.frontend, true);
  assert.equal(lock.engine, false);
  assert.equal(lock.desktop_backend, false);

  const manifest = classifyPaths(["apps/desktop/src-tauri/Cargo.toml"]);
  assert.equal(manifest.full, false);
  assert.equal(manifest.engine, false);
  assert.equal(manifest.desktop_backend, true);
});

test("includes deleted paths in routing instead of silently skipping them", () => {
  let receivedArguments;
  const files = listChangedFiles("base", "head", (...arguments_) => {
    receivedArguments = arguments_;
    return "apps/desktop/src/removed.tsx\n";
  });

  assert.deepEqual(receivedArguments, ["diff", "--name-only", "base", "head"]);
  assert.equal(classifyPaths(files).frontend, true);
});

test("recognizes an app release when every executable manifest changes only version", () => {
  const values = appReleaseValues("1.0.0", "1.0.1");
  assert.equal(isPureAppReleaseChange(APP_RELEASE_FILES, reader(values)), true);
  assert.equal(
    classifyChanges(APP_RELEASE_FILES, reader(values)).release_app,
    true,
  );
});

test("rejects an app release that leaves the remote helper behind", () => {
  const values = {
    ...appReleaseValues("1.0.0", "1.0.1"),
    "head:crates/antiburn-remote/Cargo.toml": remoteCargoToml("1.0.0"),
    "head:crates/antiburn-remote/Cargo.lock": cargoLock(
      "antiburn-remote",
      "1.0.0",
    ),
    "head:apps/desktop/src-tauri/Cargo.lock": appCargoLock("1.0.1", "1.0.0"),
  };
  assert.equal(
    isPureAppReleaseChange(APP_RELEASE_FILES, reader(values)),
    false,
  );
});

test("rejects an app release that smuggles a dependency change into Cargo files", () => {
  const values = {
    ...appReleaseValues("1.0.0", "1.0.1"),
    "head:apps/desktop/src-tauri/Cargo.toml": cargoToml("1.0.1", "2"),
    "head:apps/desktop/src-tauri/Cargo.lock": appCargoLock(
      "1.0.1",
      "1.0.1",
      "2.0.0",
    ),
  };
  assert.equal(
    isPureAppReleaseChange(APP_RELEASE_FILES, reader(values)),
    false,
  );
  assert.equal(
    classifyChanges(APP_RELEASE_FILES, reader(values)).desktop_backend,
    true,
  );
});

test("recognizes an engine release across both lockfiles", () => {
  const files = [
    "apps/desktop/src-tauri/Cargo.lock",
    "crates/antiburn-local/CHANGELOG.md",
    "crates/antiburn-local/Cargo.lock",
    "crates/antiburn-local/Cargo.toml",
  ];
  const values = {
    "base:crates/antiburn-local/Cargo.toml": cargoToml("1.0.0"),
    "head:crates/antiburn-local/Cargo.toml": cargoToml("1.0.1"),
    "base:crates/antiburn-local/Cargo.lock": cargoLock(
      "antiburn-local",
      "1.0.0",
    ),
    "head:crates/antiburn-local/Cargo.lock": cargoLock(
      "antiburn-local",
      "1.0.1",
    ),
    "base:apps/desktop/src-tauri/Cargo.lock": cargoLock(
      "antiburn-local",
      "1.0.0",
    ),
    "head:apps/desktop/src-tauri/Cargo.lock": cargoLock(
      "antiburn-local",
      "1.0.1",
    ),
  };
  assert.equal(isPureEngineReleaseChange(files, reader(values)), true);
  assert.equal(classifyChanges(files, reader(values)).release_engine, true);
});

const ALL_WORKSPACES = {
  engine: "engine,remote",
  desktop: "shell,trace,hud,main-window,anchored-window,nudge,sound,tray-icon",
};

const OUTPUT_KEYS = [
  "stage",
  "docs_only",
  "frontend",
  "engine",
  "desktop_backend",
  "full",
  "release_app",
  "release_engine",
  "rust_platforms",
  "frontend_platforms",
  "engine_workspaces",
  "desktop_workspaces",
  "run_engine",
  "run_desktop",
  "run_frontend",
  "run_ci_scripts",
  "run_installers",
  "run_licenses",
  "run_advisories",
  "run_remote_build",
  "run_slop",
  "run_no_analytics_check",
  "run_release_metadata",
  "run_warm_release_cache",
];

function plan(stage, files, release) {
  return planOutputs({ stage, changes: routeChanges(files), release });
}

function platformNames(value) {
  return JSON.parse(value).map((platform) => platform.name);
}

test("resolves the stage from the event, the draft flag, and live labels", () => {
  const cases = [
    [{ eventName: "pull_request", draft: true, labels: [] }, "draft"],
    [{ eventName: "pull_request", draft: true, labels: ["ci:full"] }, "ready"],
    [
      { eventName: "pull_request", draft: true, labels: ["bug", "ci:full"] },
      "ready",
    ],
    [{ eventName: "pull_request", draft: true, labels: ["bug"] }, "draft"],
    [{ eventName: "pull_request", draft: false, labels: [] }, "ready"],
    [{ eventName: "push", ref: "refs/heads/main" }, "main"],
    [{ eventName: "schedule", ref: "refs/heads/main" }, "nightly"],
    [{ eventName: "workflow_dispatch", ref: "refs/heads/main" }, "nightly"],
  ];
  for (const [input, expected] of cases) {
    assert.equal(resolveStage(input), expected, JSON.stringify(input));
  }
});

test("writes exactly the documented output keys as strings", () => {
  for (const stage of ["draft", "ready", "main", "nightly"]) {
    const outputs = plan(stage, ["apps/desktop/src/App.tsx"]);
    assert.deepEqual(Object.keys(outputs), OUTPUT_KEYS);
    for (const [key, value] of Object.entries(outputs)) {
      assert.equal(typeof value, "string", key);
      assert.equal(value.includes("\n"), false, key);
    }
  }
});

test("chooses platforms per stage", () => {
  const expected = {
    draft: { rust: ["linux"], frontend: ["linux"] },
    ready: {
      rust: ["linux", "windows", "macos"],
      frontend: ["linux", "windows"],
    },
    main: {
      rust: ["linux", "linux-arm64", "windows", "windows-arm64", "macos"],
      frontend: ["linux", "windows"],
    },
    nightly: {
      rust: ["linux", "linux-arm64", "windows", "windows-arm64", "macos"],
      frontend: ["linux", "windows"],
    },
  };
  for (const [stage, platforms] of Object.entries(expected)) {
    const outputs = plan(stage, ["crates/antiburn-local/src/lib.rs"]);
    assert.deepEqual(platformNames(outputs.rust_platforms), platforms.rust);
    assert.deepEqual(
      platformNames(outputs.frontend_platforms),
      platforms.frontend,
    );
  }
  assert.deepEqual(JSON.parse(plan("main", []).rust_platforms), [
    { name: "linux", os: "ubuntu-24.04" },
    { name: "linux-arm64", os: "ubuntu-24.04-arm" },
    { name: "windows", os: "windows-latest" },
    { name: "windows-arm64", os: "windows-11-arm" },
    { name: "macos", os: "macos-latest" },
  ]);
});

test("selects transitive dependents of a changed workspace", () => {
  assert.deepEqual(withDependents(["engine"]), ["engine", "remote", "shell"]);
  assert.deepEqual(withDependents(["remote"]), ["remote", "shell"]);
  assert.deepEqual(withDependents(["hud"]), ["shell", "hud"]);
  assert.deepEqual(withDependents(["tray-icon"]), ["shell", "tray-icon"]);
  assert.deepEqual(withDependents(["shell"]), ["shell"]);
});

test("routes Rust paths to their workspaces", () => {
  const cases = [
    ["crates/antiburn-local/src/lib.rs", "engine,remote", "shell"],
    ["crates/antiburn-remote/src/main.rs", "remote", "shell"],
    ["apps/desktop/src-tauri/src/lib.rs", "", "shell"],
    ["apps/desktop/src-tauri/tauri.conf.json", "", "shell"],
    ["apps/desktop/src-tauri/crates/hud/src/lib.rs", "", "shell,hud"],
    ["apps/desktop/src-tauri/crates/trace/src/lib.rs", "", "shell,trace"],
    [
      "apps/desktop/src-tauri/crates/main-window/src/lib.rs",
      "",
      "shell,main-window",
    ],
    [
      "apps/desktop/src-tauri/crates/anchored-window/src/lib.rs",
      "",
      "shell,anchored-window",
    ],
    ["apps/desktop/src-tauri/crates/nudge/src/lib.rs", "", "shell,nudge"],
    ["apps/desktop/src-tauri/crates/sound/src/lib.rs", "", "shell,sound"],
    [
      "apps/desktop/src-tauri/vendor/tray-icon/src/lib.rs",
      "",
      "shell,tray-icon",
    ],
  ];
  for (const [file, engine, desktop] of cases) {
    const outputs = plan("ready", [file]);
    assert.equal(outputs.engine_workspaces, engine, file);
    assert.equal(outputs.desktop_workspaces, desktop, file);
    assert.equal(outputs.run_engine, String(engine !== ""), file);
    assert.equal(outputs.run_desktop, "true", file);
    assert.equal(outputs.run_frontend, "false", file);
    assert.equal(outputs.run_licenses, "false", file);
    assert.equal(outputs.full, "false", file);
  }
});

test("routes a lockfile change to its workspace and the license checks", () => {
  for (const [file, engine, desktop] of [
    ["crates/antiburn-local/Cargo.lock", "engine,remote", "shell"],
    ["crates/antiburn-local/deny.toml", "engine,remote", "shell"],
    ["crates/antiburn-remote/Cargo.toml", "remote", "shell"],
    ["apps/desktop/src-tauri/Cargo.lock", "", "shell"],
    ["apps/desktop/src-tauri/about.toml", "", "shell"],
    ["apps/desktop/src-tauri/crates/hud/Cargo.lock", "", "shell,hud"],
    [
      "apps/desktop/src-tauri/vendor/tray-icon/Cargo.toml",
      "",
      "shell,tray-icon",
    ],
  ]) {
    const outputs = plan("ready", [file]);
    assert.equal(outputs.full, "false", file);
    assert.equal(outputs.engine_workspaces, engine, file);
    assert.equal(outputs.desktop_workspaces, desktop, file);
    assert.equal(outputs.run_licenses, "true", file);
    assert.equal(outputs.run_frontend, "false", file);
  }
});

test("routes frontend, Node manifest, and license inputs", () => {
  const frontend = plan("ready", ["apps/desktop/src/App.tsx"]);
  assert.equal(frontend.run_frontend, "true");
  assert.equal(frontend.run_licenses, "false");
  assert.equal(frontend.run_ci_scripts, "false");
  assert.equal(frontend.run_engine, "false");
  assert.equal(frontend.run_desktop, "false");

  const appManifest = plan("ready", ["apps/desktop/package.json"]);
  assert.equal(appManifest.run_frontend, "true");
  assert.equal(appManifest.run_licenses, "true");
  assert.equal(appManifest.run_ci_scripts, "false");

  for (const file of [
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
  ]) {
    const outputs = plan("ready", [file]);
    assert.equal(outputs.run_frontend, "true", file);
    assert.equal(outputs.run_ci_scripts, "true", file);
    assert.equal(outputs.run_licenses, "true", file);
    assert.equal(outputs.run_installers, "false", file);
    assert.equal(outputs.run_desktop, "false", file);
  }

  for (const file of [
    "THIRD_PARTY_NOTICES",
    "scripts/third-party-notices.mjs",
    "scripts/third-party-license-overrides/react-remove-scroll-bar-2.3.8.txt",
  ]) {
    const outputs = plan("ready", [file]);
    assert.equal(outputs.run_licenses, "true", file);
    assert.equal(outputs.run_frontend, "false", file);
    assert.equal(outputs.run_desktop, "false", file);
  }
});

test("routes installer files to the installer tests only", () => {
  for (const file of [
    "install.sh",
    "install.ps1",
    "scripts/install-sh.test.mjs",
    "scripts/install-ps1.test.ps1",
    "scripts/windows-signing.ps1",
    "scripts/windows-signing.test.ps1",
    "scripts/setup-windows-signing.ps1",
  ]) {
    const outputs = plan("ready", [file]);
    assert.equal(outputs.run_installers, "true", file);
    assert.equal(outputs.run_ci_scripts, "false", file);
    assert.equal(outputs.run_licenses, "false", file);
    assert.equal(outputs.run_frontend, "false", file);
    assert.equal(outputs.run_engine, "false", file);
    assert.equal(outputs.run_desktop, "false", file);
    assert.equal(outputs.full, "false", file);
  }
});

test("routes other scripts to the CI script tests only", () => {
  const outputs = plan("ready", ["scripts/wait-for-main-ci.mjs"]);
  assert.equal(outputs.run_ci_scripts, "true");
  assert.equal(outputs.full, "false");
  assert.equal(outputs.run_frontend, "false");
  assert.equal(outputs.run_engine, "false");
  assert.equal(outputs.run_desktop, "false");
  assert.equal(outputs.run_installers, "false");
  assert.equal(outputs.run_licenses, "false");
});

test("selects every job for workflow changes", () => {
  const outputs = plan("ready", [".github/workflows/ci.yml"]);
  assert.equal(outputs.full, "true");
  assert.equal(outputs.engine_workspaces, ALL_WORKSPACES.engine);
  assert.equal(outputs.desktop_workspaces, ALL_WORKSPACES.desktop);
  for (const key of [
    "run_frontend",
    "run_ci_scripts",
    "run_installers",
    "run_licenses",
    "run_remote_build",
    "run_slop",
  ]) {
    assert.equal(outputs[key], "true", key);
  }
  assert.equal(outputs.run_advisories, "false");
  assert.equal(outputs.run_no_analytics_check, "false");
});

test("selects nothing but the slop gate for documentation", () => {
  const outputs = plan("ready", ["README.md", "docs/runbooks/release.md"]);
  assert.equal(outputs.docs_only, "true");
  for (const [key, value] of Object.entries(outputs)) {
    if (key.startsWith("run_") && key !== "run_slop") {
      assert.equal(value, "false", key);
    }
  }
  assert.equal(outputs.run_slop, "true");
  assert.equal(outputs.engine_workspaces, "");
  assert.equal(outputs.desktop_workspaces, "");
});

test("a draft keeps the selected workspaces but skips installers, licenses, and slop", () => {
  const outputs = plan("draft", [".github/workflows/ci.yml"]);
  assert.equal(outputs.full, "true");
  assert.equal(outputs.engine_workspaces, ALL_WORKSPACES.engine);
  assert.equal(outputs.desktop_workspaces, ALL_WORKSPACES.desktop);
  assert.equal(outputs.run_frontend, "true");
  assert.equal(outputs.run_ci_scripts, "true");
  assert.equal(outputs.run_installers, "false");
  assert.equal(outputs.run_licenses, "false");
  assert.equal(outputs.run_slop, "false");
  assert.equal(outputs.run_remote_build, "false");
  assert.equal(outputs.run_no_analytics_check, "false");
  assert.equal(outputs.run_advisories, "false");
});

test("builds the remote helper on main for engine changes and on ready for helper changes", () => {
  assert.equal(
    plan("ready", ["crates/antiburn-local/src/lib.rs"]).run_remote_build,
    "false",
  );
  assert.equal(
    plan("ready", ["crates/antiburn-remote/src/main.rs"]).run_remote_build,
    "true",
  );
  assert.equal(
    plan("draft", ["crates/antiburn-remote/src/main.rs"]).run_remote_build,
    "false",
  );
  assert.equal(
    plan("main", ["crates/antiburn-local/src/lib.rs"]).run_remote_build,
    "true",
  );
  assert.equal(
    plan("main", ["apps/desktop/src-tauri/src/lib.rs"]).run_remote_build,
    "false",
  );
});

test("runs the shell Clippy pass without analytics only on main and nightly", () => {
  const shell = ["apps/desktop/src-tauri/src/lib.rs"];
  assert.equal(plan("draft", shell).run_no_analytics_check, "false");
  assert.equal(plan("ready", shell).run_no_analytics_check, "false");
  assert.equal(plan("main", shell).run_no_analytics_check, "true");
  assert.equal(plan("nightly", []).run_no_analytics_check, "true");
  assert.equal(
    plan("main", ["apps/desktop/src/App.tsx"]).run_no_analytics_check,
    "false",
  );
});

test("nightly selects everything regardless of the diff", () => {
  for (const files of [[], ["README.md"], ["apps/desktop/src/App.tsx"]]) {
    const outputs = plan("nightly", files, { app: true });
    assert.equal(outputs.stage, "nightly");
    assert.equal(outputs.full, "true");
    assert.equal(outputs.docs_only, "false");
    assert.equal(outputs.engine_workspaces, ALL_WORKSPACES.engine);
    assert.equal(outputs.desktop_workspaces, ALL_WORKSPACES.desktop);
    for (const [key, value] of Object.entries(outputs)) {
      if (!key.startsWith("run_")) continue;
      const expected = [
        "run_release_metadata",
        "run_warm_release_cache",
      ].includes(key)
        ? "false"
        : "true";
      assert.equal(value, expected, key);
    }
    assert.equal(outputs.release_app, "false");
  }
});

test("a release bump selects only release metadata", () => {
  const app = plan("main", APP_RELEASE_FILES, { app: true, engine: false });
  assert.equal(app.release_app, "true");
  assert.equal(app.run_release_metadata, "true");
  assert.equal(app.run_warm_release_cache, "true");
  assert.equal(app.full, "false");
  assert.equal(app.engine_workspaces, "");
  assert.equal(app.desktop_workspaces, "");
  for (const key of [
    "run_engine",
    "run_desktop",
    "run_frontend",
    "run_ci_scripts",
    "run_installers",
    "run_licenses",
    "run_remote_build",
    "run_no_analytics_check",
  ]) {
    assert.equal(app[key], "false", key);
  }

  const readyApp = plan("ready", APP_RELEASE_FILES, { app: true });
  assert.equal(readyApp.run_release_metadata, "true");
  assert.equal(readyApp.run_warm_release_cache, "false");

  const engine = plan("main", ["crates/antiburn-local/Cargo.toml"], {
    app: false,
    engine: true,
  });
  assert.equal(engine.release_engine, "true");
  assert.equal(engine.run_release_metadata, "true");
  assert.equal(engine.run_warm_release_cache, "false");
  assert.equal(engine.run_engine, "false");
});
