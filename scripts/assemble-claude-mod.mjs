#!/usr/bin/env node
import {
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE = "plugins/antiburn-live";
export const PUBLIC = "plugins/antiburn-mod";
export const LOCAL = ".agent-artifacts/claude-mod";
const RELEASE_FILES = [
  ".claude-plugin/plugin.json",
  "README.md",
  "CHANGELOG.md",
  "PRIVACY.md",
  "hooks/register.tsx",
  "hooks/account-usage.ts",
  "hooks/checking-status.ts",
  "hooks/usage-graphics.ts",
  "types/index.d.ts",
];
const SHARED_FILES = [
  "hooks/brand.ts",
  "hooks/usage.ts",
  "hooks/usage-presentation.ts",
  "hooks/usage-font.ts",
  "hooks/usage-font-data.ts",
  "hooks/usage-target.tsx",
  "hooks/hooks.json",
  "types/claude-code.d.ts",
  "assets/bitcount/OFL.txt",
  "assets/bitcount/README.md",
];
const json = (value) => Buffer.from(`${JSON.stringify(value, null, 2)}\n`);

function containedFile(root, path) {
  const parts = path.split("/");
  let current = root;
  for (const part of parts) {
    if (!part || part === "." || part === "..")
      throw new Error(`Unsafe path: ${path}`);
    current = join(current, part);
    if (lstatSync(current).isSymbolicLink())
      throw new Error(`Symbolic link: ${path}`);
  }
  if (!lstatSync(current).isFile())
    throw new Error(`Not a regular file: ${path}`);
  return readFileSync(current);
}

export function releaseFiles(root = ROOT) {
  const files = new Map();
  for (const path of RELEASE_FILES)
    files.set(path, containedFile(root, `${SOURCE}/release/${path}`));
  for (const path of SHARED_FILES)
    files.set(path, containedFile(root, `${SOURCE}/${path}`));
  files.set("LICENSE", containedFile(root, "LICENSE"));
  files.set(
    "assets/icon.png",
    containedFile(root, "apps/desktop/src-tauri/icons/icon.png"),
  );
  const manifest = JSON.parse(files.get(".claude-plugin/plugin.json"));
  if (manifest.name !== "antiburn-mod")
    throw new Error("Unexpected public plugin identity");
  assertImports(files);
  return files;
}

export function assertImports(files) {
  for (const [path, bytes] of files) {
    if (!/\.(?:ts|tsx)$/.test(path)) continue;
    const source = bytes.toString();
    for (const match of source.matchAll(
      /(?:^\s*(?:import(?:\s+type)?\s+(?:[\w*\s{},]+?\s+from\s+)?|export\s+(?:type\s+)?(?:\*|\{[^}]*\})\s+from\s+)|\bimport\s*\(\s*)(['"])([^'"]+)\1/gm,
    )) {
      const target = match[2];
      if (target === "claude-code" || target.startsWith("claude-code/"))
        continue;
      if (!target.startsWith("."))
        throw new Error(`Unapproved import in ${path}: ${target}`);
      const normalized = relative(
        "/artifact",
        resolve("/artifact", dirname(path), target),
      );
      if (normalized.startsWith(".."))
        throw new Error(`Escaped import in ${path}: ${target}`);
      if (
        ![
          normalized,
          `${normalized}.ts`,
          `${normalized}.tsx`,
          `${normalized}.d.ts`,
          `${normalized}/index.ts`,
          `${normalized}/index.tsx`,
          `${normalized}/index.d.ts`,
        ].some((name) => files.has(name))
      ) {
        throw new Error(`Missing or escaped import in ${path}: ${target}`);
      }
    }
  }
}

function listFiles(root, prefix = "") {
  const found = [];
  for (const entry of readdirSync(join(root, prefix), {
    withFileTypes: true,
  })) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isSymbolicLink()) throw new Error(`Symbolic link: ${path}`);
    if (entry.isDirectory()) found.push(...listFiles(root, path));
    else if (entry.isFile()) found.push(path);
    else throw new Error(`Not a regular file: ${path}`);
  }
  return found.sort();
}

export function checkTree(destination, expected) {
  if (lstatSync(destination).isSymbolicLink())
    throw new Error(`Symbolic link: ${destination}`);
  const actual = listFiles(destination);
  const missing = [...expected.keys()].filter((path) => !actual.includes(path));
  const extra = actual.filter((path) => !expected.has(path));
  const changed = actual.filter(
    (path) =>
      expected.has(path) &&
      !readFileSync(join(destination, path)).equals(expected.get(path)),
  );
  if (missing.length || extra.length || changed.length) {
    throw new Error(
      `Artifact drift: ${JSON.stringify({ missing, extra, changed })}`,
    );
  }
}

function containedDestination(root, destination) {
  const path = relative(root, destination);
  if (!path || path.startsWith(".."))
    throw new Error(`Unsafe destination: ${destination}`);
  let current = root;
  for (const part of path.split("/")) {
    current = join(current, part);
    if (lstatSync(current, { throwIfNoEntry: false })?.isSymbolicLink())
      throw new Error(`Symbolic link: ${current}`);
  }
}

function writeTree(root, destination, files) {
  containedDestination(root, destination);
  if (lstatSync(destination, { throwIfNoEntry: false })?.isSymbolicLink())
    throw new Error(`Symbolic link: ${destination}`);
  if (lstatSync(destination, { throwIfNoEntry: false })) listFiles(destination);
  rmSync(destination, { recursive: true, force: true });
  for (const [path, bytes] of [...files].sort(([a], [b]) =>
    a.localeCompare(b),
  )) {
    const target = join(destination, path);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, bytes);
  }
}

export function rewriteDevSource(source) {
  return source
    .replaceAll("antiburn-live", "antiburn-mod-dev")
    .replace(/(^|[\s`'"])\/antiburn(?=$|[\s`'".,!?])/gm, "$1/antiburn-mod-dev")
    .replace(
      /(name:\s*|command:\s*|let commandName =\s*)(['"])antiburn\2/g,
      "$1$2antiburn-mod-dev$2",
    );
}

function devFiles(root, tests = false) {
  const files = new Map();
  const sourceRoot = join(root, SOURCE);
  for (const path of listFiles(sourceRoot)) {
    if (
      path.startsWith("release/") ||
      path.startsWith(".claude-plugin/types/") ||
      (!tests && /\.test\.tsx?$/.test(path)) ||
      path === "tsconfig.json"
    )
      continue;
    const bytes = containedFile(sourceRoot, path);
    files.set(
      path,
      /\.(?:ts|tsx|md|json)$/.test(path)
        ? Buffer.from(rewriteDevSource(bytes.toString()))
        : bytes,
    );
  }
  const manifest = JSON.parse(files.get(".claude-plugin/plugin.json"));
  manifest.name = "antiburn-mod-dev";
  manifest.displayName = "antiburn (Mod · Dev)";
  files.set(".claude-plugin/plugin.json", json(manifest));
  files.set("LICENSE", containedFile(root, "LICENSE"));
  assertImports(files);
  return files;
}

export function assertValidation(report, type = "plugin") {
  if (!report.success || report.manifest?.type !== type)
    throw new Error("Host validation failed or returned an unexpected report");

  for (const entry of [report.manifest, ...(report.contents ?? [])]) {
    if ((entry.errors ?? []).length)
      throw new Error(`Validation errors: ${JSON.stringify(entry.errors)}`);
    if ((entry.warnings ?? []).length)
      throw new Error(
        `Unexpected validation warning: ${JSON.stringify(entry.warnings)}`,
      );
  }
}

export function assertCapabilities(report) {
  if (!report.success) throw new Error("Host validation failed");
  const inventories = (report.contents ?? [])
    .flatMap((entry) => entry.notes ?? [])
    .filter((note) => note.includes(" calls:"));
  if (inventories.length === 0)
    throw new Error("Host validation did not report a call inventory");
  const allowed = new Set([
    "$.clock.now",
    "$.command.register",
    "$.session.usage",
    "$.state.get",
    "$.state.set",
    "$.ui.resolve",
  ]);
  const accountOnly = new Set(["$.session.authorize", "$.http.fetch"]);
  const websiteOnly = new Set(["$.env.get", "$.process.run", "$.ui.status"]);
  const calls = inventories.flatMap((note) =>
    [
      ...note.matchAll(
        /(\$\.[A-Za-z_]+\.[A-Za-z_]+)(.*?)(?=,\s*\$\.[A-Za-z_]+\.[A-Za-z_]+|$)/g,
      ),
    ].map((match) => ({ name: match[1], attribution: match[2].trim() })),
  );
  if (!calls.some(({ name }) => name === "$.session.usage"))
    throw new Error("Usage call is missing from the host inventory");
  for (const { name, attribution } of calls) {
    if (websiteOnly.has(name)) {
      if (attribution !== "(via openTerminalWebsite)")
        throw new Error(
          `Unapproved release capability: ${name} ${attribution}`,
        );
    } else if (accountOnly.has(name)) {
      if (attribution !== "(via fetchAccountUsage)")
        throw new Error(
          `Unapproved release capability: ${name} ${attribution}`,
        );
    } else if (!allowed.has(name)) {
      throw new Error(`Unapproved release capability: ${name}`);
    }
  }
}

export function assemble(root = ROOT, mode = "release") {
  if (mode === "dev" || mode === "dev-test") {
    const destination = join(root, LOCAL, mode);
    writeTree(
      root,
      join(destination, "plugins/antiburn-mod-dev"),
      devFiles(root, mode === "dev-test"),
    );
    const marketplace = {
      name: "antiburn-local-dev",
      owner: { name: "antiburn" },
      plugins: [
        {
          name: "antiburn-mod-dev",
          source: "./plugins/antiburn-mod-dev",
        },
      ],
    };
    containedDestination(
      root,
      join(destination, ".claude-plugin/marketplace.json"),
    );
    mkdirSync(join(destination, ".claude-plugin"), { recursive: true });
    writeFileSync(
      join(destination, ".claude-plugin/marketplace.json"),
      json(marketplace),
    );
    return destination;
  }
  const files = releaseFiles(root);
  const destination = join(root, PUBLIC);
  if (mode === "check") checkTree(destination, files);
  else if (mode === "test") {
    checkTree(destination, files);
    for (const [source, target] of [
      [`${SOURCE}/release/hooks/register.test.ts`, "hooks/register.test.ts"],
      [
        `${SOURCE}/release/hooks/usage-graphics.test.ts`,
        "hooks/usage-graphics.test.ts",
      ],
      [`${SOURCE}/hooks/usage.test.ts`, "hooks/usage.test.ts"],
    ])
      files.set(target, containedFile(root, source));
    const tests = join(root, LOCAL, "test");
    writeTree(root, tests, files);
    return tests;
  } else writeTree(root, destination, files);
  return destination;
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const [flag, path, ...rest] = process.argv.slice(2);
    if (
      ["--validate-log", "--validate-marketplace-log"].includes(flag) &&
      path &&
      rest.length === 0
    ) {
      const report = JSON.parse(readFileSync(path, "utf8"));
      assertValidation(
        report,
        flag === "--validate-marketplace-log" ? "marketplace" : "plugin",
      );
      if (flag === "--validate-log") assertCapabilities(report);
      console.log("Strict validation and release capability policy passed");
    } else {
      if (
        path ||
        (flag && !["--check", "--dev", "--dev-test", "--test"].includes(flag))
      )
        throw new Error(
          "Usage: assemble-claude-mod.mjs [--check | --dev | --dev-test | --test | --validate-log report.json | --validate-marketplace-log report.json]",
        );
      console.log(assemble(ROOT, flag?.slice(2) ?? "release"));
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
