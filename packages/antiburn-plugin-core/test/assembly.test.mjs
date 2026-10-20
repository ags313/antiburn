import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtemp, realpath, readFile, writeFile, rm, mkdir, symlink } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { assemble, verify } from '../../../scripts/assemble-antiburn-shell.mjs'

test('assembly is reproducible, self-contained, licensed, and refuses altered ownership', async () => {
  const temp = await realpath(await mkdtemp(join(tmpdir(), 'antiburn-assembly-test-')))
  try {
    const output = join(temp, 'bundles')
    await assemble(output)
    const first = await readFile(join(output, 'SHA256SUMS'), 'utf8')
    assert.match(first, /claude\/hooks\/register.ts/)
    assert.match(first, /pi\/extensions\/antiburn.mjs/)
    assert.doesNotMatch(first, /claude\/lib\/(?:discovery|probe)\.mjs|\.test\./)
    await assemble(output)
    assert.equal(await readFile(join(output, 'SHA256SUMS'), 'utf8'), first)
    await verify(output)
    for (const file of [
      'codex/skills/antiburn/SKILL.md',
      'cursor/skills/antiburn/SKILL.md',
      'opencode/commands/antiburn.md',
    ]) {
      const command = await readFile(join(output, file), 'utf8')
      assert.match(
        command,
        /On the first help invocation only, display the verified availability result/,
      )
      assert.match(command, /Do not repeat the check or notice for subsequent help/)
    }
    const cursorSkill = await readFile(join(output, 'cursor/skills/antiburn/SKILL.md'), 'utf8')
    assert.match(cursorSkill, /name: antiburn\ndisable-model-invocation: true/)
    assert.match(cursorSkill, /Usage: \/antiburn /)
    assert.match(cursorSkill, /fenced text code block/)
    assert.match(cursorSkill, /🔥 `antiburn`/)
    assert.match(cursorSkill, /Keep the heading outside the block/)
    assert.doesNotMatch(cursorSkill, /♨|unless supplied by the host/)
    for (const host of ['claude', 'codex', 'cursor', 'opencode', 'pi'])
      assert.match(await readFile(join(output, host, 'LICENSE'), 'utf8'), /MIT License/)
    const claudeEntry = await readFile(join(output, 'claude/hooks/register.ts'), 'utf8')
    assert.doesNotMatch(claudeEntry, /packages\/|\/Users\//)
    assert.match(claudeEntry, /\.\/quota-controller/)
    const releaseEntry = await readFile(join(output, 'claude/hooks/register.ts'), 'utf8')
    const catalogEntry = await readFile(
      join(output, 'catalogs/claude/plugins/antiburn/hooks/register.ts'),
      'utf8',
    )
    assert.equal(catalogEntry, releaseEntry)
    assert.match(first, /catalogs\/claude\/plugins\/antiburn\/lib\/product\.mjs/)
    const catalog = JSON.parse(
      await readFile(join(output, 'catalogs/codex/.agents/plugins/marketplace.json'), 'utf8'),
    )
    assert.equal(catalog.plugins[0].source.path, './plugins/antiburn')
    assert.ok(
      await readFile(
        join(output, 'catalogs/codex', catalog.plugins[0].source.path, 'plugin.json'),
      ),
    )
    await writeFile(join(output, 'codex/skills/antiburn/SKILL.md'), 'user change')
    await assert.rejects(verify(output), /hash/)
    await assert.rejects(assemble(output), /hash/)
  } finally {
    await rm(temp, { recursive: true, force: true })
  }
})

test('assembly refuses unowned output and symlink destinations', async () => {
  const temp = await realpath(await mkdtemp(join(tmpdir(), 'antiburn-assembly-test-')))
  try {
    const existing = join(temp, 'existing')
    await mkdir(existing)
    await writeFile(join(existing, 'mine'), 'keep')
    await assert.rejects(assemble(existing))
    const link = join(temp, 'link')
    await symlink(existing, link)
    await assert.rejects(assemble(link), /symlink/)
    assert.equal(await readFile(join(existing, 'mine'), 'utf8'), 'keep')
  } finally {
    await rm(temp, { recursive: true, force: true })
  }
})
