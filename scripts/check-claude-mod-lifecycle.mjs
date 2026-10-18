#!/usr/bin/env node
import assert from "node:assert/strict";
import {
  cpSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import { checkTree, releaseFiles } from "./assemble-claude-mod.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temporary = mkdtempSync(join(tmpdir(), "antiburn-mod-lifecycle-"));
const profile = join(temporary, "profile");
const project = join(temporary, "project");
mkdirSync(profile);
mkdirSync(project);
const release = "antiburn-mod@antiburn";
const dev = "antiburn-mod-dev@antiburn-local-dev";

function run(...args) {
  const result = spawnSync("claude", args, {
    cwd: project,
    env: { ...process.env, CLAUDE_CONFIG_DIR: profile },
    encoding: "utf8",
    timeout: 30000,
  });
  if (result.error || result.status !== 0) {
    throw new Error(
      `claude ${args.join(" ")} failed: ${result.error?.message ?? result.stderr ?? result.stdout}`,
    );
  }
  return result.stdout.trim();
}

function installed() {
  const listing = JSON.parse(run("plugin", "list", "--json"));
  assert.ok(Array.isArray(listing), "Unexpected plugin list schema");
  return listing;
}

function record(id) {
  return installed().find((entry) => entry.id === id);
}

try {
  assert.deepEqual(installed(), []);
  const localCatalog = join(temporary, "local-catalog");
  cpSync(
    join(root, "plugins/antiburn-mod"),
    join(localCatalog, "plugins/antiburn-mod"),
    { recursive: true },
  );
  mkdirSync(join(localCatalog, ".claude-plugin"), { recursive: true });
  const localManifest = JSON.parse(
    readFileSync(join(root, ".claude-plugin/marketplace.json"), "utf8"),
  );
  localManifest.plugins[0].source = "./plugins/antiburn-mod";
  writeFileSync(
    join(localCatalog, ".claude-plugin/marketplace.json"),
    JSON.stringify(localManifest),
  );
  run("plugin", "marketplace", "add", localCatalog);
  run(
    "plugin",
    "marketplace",
    "add",
    join(root, ".agent-artifacts/claude-mod/dev"),
  );
  run("plugin", "install", release, "--scope", "user", "--json");
  run("plugin", "install", dev, "--scope", "user", "--json");
  assert.equal(record(release)?.enabled, true);
  assert.equal(record(dev)?.enabled, true);
  assert.equal(installed().length, 2);
  run("plugin", "disable", dev, "--scope", "user");
  assert.equal(record(dev)?.enabled, false);
  assert.equal(record(release)?.enabled, true);
  run("plugin", "enable", dev, "--scope", "user");
  run("plugin", "update", dev, "--scope", "user");
  assert.equal(record(release)?.enabled, true);
  run("plugin", "uninstall", dev, "--scope", "user");
  assert.equal(record(dev), undefined);
  assert.equal(record(release)?.enabled, true);
  run("plugin", "install", dev, "--scope", "user", "--json");
  run("plugin", "update", release, "--scope", "user");
  run("plugin", "uninstall", release, "--scope", "user");
  assert.equal(record(release), undefined);
  assert.equal(record(dev)?.enabled, true);
  run("plugin", "uninstall", dev, "--scope", "user");
  assert.deepEqual(installed(), []);
  console.log(
    "Local marketplace lifecycle passed: independent install, enable, disable, update, uninstall and reinstall in an isolated profile.",
  );
  run("plugin", "marketplace", "remove", "antiburn");
  const fixture = join(temporary, "fixture.git");
  const catalog = join(temporary, "catalog");
  cpSync(join(root, "plugins/antiburn-mod"), fixture, { recursive: true });
  mkdirSync(join(catalog, ".claude-plugin"), { recursive: true });
  const manifestPath = join(fixture, ".claude-plugin/plugin.json");
  const originalManifest = readFileSync(manifestPath);
  const candidateVersion = JSON.parse(originalManifest).version;
  const versionParts = /^(\d+)\.(\d+)\.(\d+)$/.exec(candidateVersion);
  assert.ok(
    versionParts,
    "The candidate needs a numeric major.minor.patch version",
  );
  const updateVersion = `${versionParts[1]}.${versionParts[2]}.${BigInt(versionParts[3]) + 1n}`;
  const originalReadme = readFileSync(join(fixture, "README.md"));
  function git(...args) {
    const result = spawnSync("git", args, {
      cwd: fixture,
      encoding: "utf8",
      timeout: 30000,
    });
    assert.equal(result.status, 0, result.stderr || result.error?.message);
  }
  function commit(message) {
    git("add", ".");
    git(
      "-c",
      "user.name=Test Fixture",
      "-c",
      "user.email=fixture@example.invalid",
      "commit",
      "-s",
      "-m",
      message,
    );
  }
  git("init", "-b", "main");
  commit("Initial disposable release fixture");
  writeFileSync(
    join(catalog, ".claude-plugin/marketplace.json"),
    JSON.stringify({
      name: "antiburn",
      owner: { name: "antiburn" },
      plugins: [
        {
          name: "antiburn-mod",
          source: { source: "url", url: pathToFileURL(fixture).href },
        },
      ],
    }),
  );
  run("plugin", "marketplace", "add", catalog);
  run("plugin", "install", release, "--scope", "user", "--json");
  const initial = record(release);
  assert.equal(initial.version, candidateVersion);
  assert.ok(initial.installPath.startsWith(join(profile, "plugins/cache/")));
  checkTree(initial.installPath, releaseFiles(root));
  writeFileSync(join(fixture, "README.md"), "Disposable update fixture\n");
  writeFileSync(
    manifestPath,
    JSON.stringify({ ...JSON.parse(originalManifest), version: updateVersion }),
  );
  commit("Disposable update fixture");
  assert.deepEqual(
    readFileSync(join(initial.installPath, "README.md")),
    originalReadme,
  );
  run("plugin", "update", release, "--scope", "user");
  assert.equal(record(release).version, updateVersion);
  assert.equal(
    readFileSync(join(record(release).installPath, "README.md"), "utf8"),
    "Disposable update fixture\n",
  );
  writeFileSync(manifestPath, originalManifest);
  writeFileSync(join(fixture, "README.md"), originalReadme);
  commit("Restore known good disposable release fixture");
  run("plugin", "uninstall", release, "--scope", "user");
  run("plugin", "install", release, "--scope", "user", "--json");
  assert.equal(record(release).version, candidateVersion);
  checkTree(record(release).installPath, releaseFiles(root));
  run("plugin", "uninstall", release, "--scope", "user");
  assert.deepEqual(installed(), []);
  console.log(
    `Cached git-source rehearsal passed: exact candidate bytes, cache isolation, ${candidateVersion} to ${updateVersion} update, and explicit uninstall/reinstall rollback to ${candidateVersion}.`,
  );
  console.log(
    "Git fixtures use local file URLs. This does not prove hosted transport, native rendering, or in-session state migration.",
  );
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
