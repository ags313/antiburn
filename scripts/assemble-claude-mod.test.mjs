import assert from "node:assert/strict";
import {
  cpSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import {
  assemble,
  assertCapabilities,
  assertImports,
  assertValidation,
  checkTree,
  releaseFiles,
  rewriteDevSource,
} from "./assemble-claude-mod.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const report = (...calls) => ({
  success: true,
  contents: [{ notes: [`./register.tsx calls: ${calls.join(", ")}`] }],
});

function temporary(t) {
  const path = mkdtempSync(join(tmpdir(), "antiburn-packaging-"));
  t.after(() => rmSync(path, { recursive: true, force: true }));
  return path;
}

test("release allowlist is self-contained and preserves authoritative assets", () => {
  const files = releaseFiles(root);
  assert.equal(files.size, 21);
  assert.deepEqual(
    files.get("hooks/checking-status.ts"),
    readFileSync(
      join(root, "plugins/antiburn-live/release/hooks/checking-status.ts"),
    ),
  );
  assert.deepEqual(files.get("LICENSE"), readFileSync(join(root, "LICENSE")));
  assert.deepEqual(
    files.get("assets/icon.png"),
    readFileSync(join(root, "apps/desktop/src-tauri/icons/icon.png")),
  );
  for (const path of files.keys())
    assert.doesNotMatch(
      path,
      /(?:checks|catalog|burn-controls|terminal-flame|fixtures|\.test\.|\.claude-plugin\/types)/,
    );
  const manifest = JSON.parse(files.get(".claude-plugin/plugin.json"));
  assert.equal(manifest.name, "antiburn-mod");
  assert.equal(manifest.version, "0.1.2");
  const marketplace = JSON.parse(
    readFileSync(join(root, ".claude-plugin/marketplace.json")),
  );
  assert.equal(marketplace.plugins.length, 1);
  assert.equal(marketplace.plugins[0].name, manifest.name);
  assert.deepEqual(marketplace.plugins[0].source, {
    source: "git-subdir",
    url: "https://github.com/antiburn/antiburn.git",
    path: "plugins/antiburn-mod",
    sha: "6de23173eb1c6f7a51381fa79f69cb4a7a09885a",
  });
  assert.equal(marketplace.plugins[0].version, undefined);
  assertImports(files);
});

test("tree verification rejects stale, omitted, and unexpected files without writing", (t) => {
  const path = temporary(t);
  const expected = new Map([
    ["one", Buffer.from("first")],
    ["two", Buffer.from("second")],
  ]);
  writeFileSync(join(path, "one"), "first");
  writeFileSync(join(path, "two"), "second");
  checkTree(path, expected);
  writeFileSync(join(path, "one"), "changed");
  rmSync(join(path, "two"));
  writeFileSync(join(path, "private"), "unexpected");
  assert.throws(
    () => checkTree(path, expected),
    /"missing":\["two"\].*"extra":\["private"\].*"changed":\["one"\]/,
  );
  assert.equal(readFileSync(join(path, "one"), "utf8"), "changed");
});

test("tree verification rejects symlinks", (t) => {
  const path = temporary(t);
  symlinkSync(resolve(root, "LICENSE"), join(path, "LICENSE"));
  assert.throws(() => checkTree(path, new Map()), /Symbolic link/);
});

test("source collection rejects a symlink in a parent directory", (t) => {
  const path = temporary(t);
  mkdirSync(join(path, "plugins"));
  symlinkSync(
    join(root, "plugins/antiburn-live"),
    join(path, "plugins/antiburn-live"),
  );
  assert.throws(() => releaseFiles(path), /Symbolic link/);
});

test("import verification rejects omitted modules, package imports, and path escapes", () => {
  for (const source of [
    "import x from './checks'",
    "import x from 'node:fs'",
    "import x from '../../usage'",
    "const x = import('./checks')",
    "import './checks'",
  ]) {
    assert.throws(
      () =>
        assertImports(
          new Map([
            ["hooks/register.tsx", Buffer.from(source)],
            ["usage.ts", Buffer.from("export {}")],
          ]),
        ),
      /import/i,
    );
  }
  assertImports(
    new Map([
      [
        "hooks/register.tsx",
        Buffer.from("import { on } from 'claude-code'; import './usage'"),
      ],
      ["hooks/usage.ts", Buffer.from("export {}")],
    ]),
  );
});

test("development rewrite isolates state and both registrations without replacing returned routing", () => {
  const source = `const owner = { plugin: 'antiburn-live' }
let commandName = 'antiburn'
const registered = await $.command.register({ name: 'antiburn' })
commandName = registered.command
const again = await $.command.register({ name: "antiburn" })
commandName = again.command
const label = 'antiburn'`;
  const rewritten = rewriteDevSource(source);
  assert.doesNotMatch(rewritten, /antiburn-live/);
  assert.equal(rewritten.match(/antiburn-mod-dev/g).length, 4);
  assert.match(rewritten, /commandName = registered\.command/);
  assert.match(rewritten, /commandName = again\.command/);
  assert.match(rewritten, /const label = 'antiburn'/);
});

test("capability policy permits website activation APIs and rejects background and model actions", () => {
  assertCapabilities(
    report(
      "$.session.usage",
      "$.clock.now",
      "$.state.get",
      "$.command.register",
      "$.env.get (via openTerminalWebsite)",
      "$.process.run (via openTerminalWebsite)",
      "$.ui.status (via openTerminalWebsite)",
    ),
  );
  for (const call of [
    "$.clock.after",
    "$.clock.every",
    "$.http.get",
    "$.model.query",
    "$.prompt.fill",
    "$.prompt.submit",
    "$.session.compact",
    "$.command.list",
  ]) {
    assert.throws(
      () => assertCapabilities(report("$.session.usage", call)),
      /Unapproved release capability/,
    );
  }
  assert.throws(() => assertCapabilities({ success: true }), /inventory/);
  assert.throws(() => assertCapabilities({ success: false }), /failed/);
  assert.throws(() => assertCapabilities(report("$.clock.now")), /Usage call/);
});

test("website capabilities require exclusive opener attribution for every occurrence", () => {
  for (const name of ["$.env.get", "$.process.run", "$.ui.status"]) {
    for (const attribution of [
      "",
      " (via sessionStart)",
      " (via openTerminalWebsite, sessionStart)",
      " (via sessionStart, openTerminalWebsite)",
      " (via openTerminalWebsiteExtra)",
      " (via openTerminalWebsite) (via sessionStart)",
    ]) {
      assert.throws(
        () =>
          assertCapabilities(
            report("$.session.usage", `${name}${attribution}`),
          ),
        /Unapproved release capability/,
      );
    }
    assert.throws(
      () =>
        assertCapabilities(
          report("$.session.usage", `${name} (via openTerminalWebsite)`, name),
        ),
      /Unapproved release capability/,
    );
    const mixed = report(
      "$.session.usage",
      `${name} (via openTerminalWebsite)`,
    );
    mixed.contents.push({ notes: [`./other.ts calls: ${name}`] });
    assert.throws(
      () => assertCapabilities(mixed),
      /Unapproved release capability/,
    );
  }
});

test("validation policy rejects every warning and error", () => {
  const warning = {
    path: "name",
    code: null,
    message:
      "Plugin name \"antiburn-mod\" reads as one of Anthropic's own. Name it for what it does. If this is one of Anthropic's own plugins, validate the marketplace that lists it.",
  };
  const base = {
    success: true,
    manifest: { type: "plugin", errors: [], warnings: [] },
    contents: [],
  };
  assertValidation({
    ...base,
    manifest: { type: "plugin", errors: [], warnings: [] },
  });
  assert.throws(
    () =>
      assertValidation({
        ...base,
        manifest: { ...base.manifest, warnings: [warning] },
      }),
    /Unexpected validation warning/,
  );
  for (const changed of [
    { path: "author" },
    { message: `${warning.message} Extra warning.` },
    { code: "new-code" },
  ]) {
    assert.throws(
      () =>
        assertValidation({
          ...base,
          manifest: {
            ...base.manifest,
            warnings: [{ ...warning, ...changed }],
          },
        }),
      /Unexpected validation warning/,
    );
  }
  assert.throws(
    () =>
      assertValidation({ ...base, contents: [{ errors: ["broken import"] }] }),
    /Validation errors/,
  );
  assert.throws(
    () => assertValidation({ ...base, contents: [{ warnings: [warning] }] }),
    /Unexpected validation warning/,
  );
  assert.throws(
    () => assertValidation({ ...base, success: false }),
    /Host validation failed/,
  );
});

test("assembly is deterministic and isolates private, development, and test files", (t) => {
  const path = temporary(t);
  mkdirSync(join(path, "plugins"), { recursive: true });
  cpSync(
    join(root, "plugins/antiburn-live"),
    join(path, "plugins/antiburn-live"),
    {
      recursive: true,
      filter: (source) => !source.includes("/.claude-plugin/types"),
    },
  );
  const privateTypes = join(path, "plugins/antiburn-live/.claude-plugin/types");
  mkdirSync(privateTypes, { recursive: true });
  writeFileSync(join(privateTypes, "account.d.ts"), "private fixture");
  mkdirSync(join(path, "apps/desktop/src-tauri/icons"), { recursive: true });
  cpSync(
    join(root, "apps/desktop/src-tauri/icons/icon.png"),
    join(path, "apps/desktop/src-tauri/icons/icon.png"),
  );
  cpSync(join(root, "LICENSE"), join(path, "LICENSE"));
  const destination = assemble(path);
  const expected = releaseFiles(path);
  checkTree(destination, expected);
  assemble(path);
  checkTree(destination, expected);
  assert.equal(existsSync(join(destination, ".claude-plugin/types")), false);
  assert.equal(existsSync(join(destination, "hooks/checks.ts")), false);
  const devRoot = assemble(path, "dev");
  const plugin = join(devRoot, "plugins/antiburn-mod-dev");
  assert.equal(existsSync(join(plugin, "release")), false);
  assert.equal(existsSync(join(plugin, ".claude-plugin/types")), false);
  assert.equal(existsSync(join(plugin, "hooks/register.test.ts")), false);
  assert.equal(existsSync(join(plugin, "hooks/checks.ts")), true);
  const dev = readFileSync(join(plugin, "hooks/register.tsx"), "utf8");
  assert.doesNotMatch(dev, /plugin: 'antiburn-live'|name: 'antiburn'/);
  assert.match(dev, /commandName = registered.command/);
  const testRoot = assemble(path, "test");
  assert.equal(existsSync(join(testRoot, "hooks/register.test.ts")), true);
  for (const [name, content] of expected)
    assert.deepEqual(readFileSync(join(testRoot, name)), content);
  rmSync(join(path, ".agent-artifacts"), { recursive: true });
  const external = temporary(t);
  symlinkSync(external, join(path, ".agent-artifacts"));
  assert.throws(() => assemble(path, "dev"), /Symbolic link/);
  assert.equal(existsSync(join(external, "claude-mod")), false);
});

test("development command help changes without changing brand URLs", () => {
  const input =
    "Run /antiburn demo off. ` /antiburn ` https://antiburn.com https://github.com/antiburn/antiburn";
  const result = rewriteDevSource(input);
  assert.equal(
    result,
    "Run /antiburn-mod-dev demo off. ` /antiburn-mod-dev ` https://antiburn.com https://github.com/antiburn/antiburn",
  );
});

test("account capabilities require exclusive fetchAccountUsage attribution", () => {
  for (const name of ["$.session.authorize", "$.http.fetch"]) {
    assertCapabilities(
      report("$.session.usage", `${name} (via fetchAccountUsage)`),
    );
    for (const attribution of [
      "",
      " (via renderUsage)",
      " (via fetchAccountUsage, readUsage)",
      " (via readUsage, fetchAccountUsage)",
      " (via fetchAccountUsageExtra)",
      " (via fetchAccountUsage) (via renderUsage)",
    ]) {
      assert.throws(
        () =>
          assertCapabilities(
            report("$.session.usage", `${name}${attribution}`),
          ),
        /Unapproved release capability/,
      );
    }
    assert.throws(
      () =>
        assertCapabilities(
          report("$.session.usage", `${name} (via fetchAccountUsage)`, name),
        ),
      /Unapproved release capability/,
    );
  }
});
