import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const audit = spawnSync('npm', ['audit', '--audit-level=high', '--json'], {
  cwd: fileURLToPath(new URL('../', import.meta.url)),
  encoding: 'utf8', maxBuffer: 10 * 1024 * 1024,
})
// Local backports do not satisfy issue #7's upstream-release and clean-audit requirements.
// Preserve npm's report and failure signal, including registry errors.
writeFileSync(new URL('../npm-audit.json', import.meta.url), audit.stdout ?? '')
if (audit.stdout) process.stdout.write(audit.stdout)
if (audit.stderr) process.stderr.write(audit.stderr)
assert.ifError(audit.error)
process.exitCode = audit.status ?? 1
