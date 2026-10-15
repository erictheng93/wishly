import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { appendFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const backports = new Map([
  ['braces', ['https://github.com/advisories/GHSA-vfj7-8cjw-p6xm', '<=3.0.3']],
  ['node-forge', ['https://github.com/advisories/GHSA-86w9-cpqp-85rv', '<=1.4.0']],
])

// Only classify advisories here. The CLI separately requires patched hashes and real security tests.
export function evaluateAudit(report, status) {
  assert(report && !report.error && report.auditReportVersion === 2, 'npm audit failed or returned an unsupported report')
  assert(report.vulnerabilities && typeof report.vulnerabilities === 'object' && !Array.isArray(report.vulnerabilities), 'Missing audit findings')
  const entries = Object.entries(report.vulnerabilities)
  for (const [, finding] of entries) {
    assert(finding && ['info', 'low', 'moderate', 'high', 'critical'].includes(finding.severity), 'Invalid audit severity')
  }
  const high = entries.filter(([, v]) => v.severity === 'high')
  const critical = entries.filter(([, v]) => v.severity === 'critical')
  assert.equal(report.metadata?.vulnerabilities?.high, high.length, 'Inconsistent high severity count')
  assert.equal(report.metadata?.vulnerabilities?.critical, critical.length, 'Inconsistent critical severity count')
  assert.equal(status, high.length + critical.length > 0 ? 1 : 0, 'Unexpected npm audit exit status')
  assert.equal(critical.length, 0, 'Critical advisories are never covered by these backports')

  const accepted = new Set()
  for (const [name] of high) {
    const pending = [name]
    const visited = new Set()
    let covered = false
    while (pending.length) {
      const dependency = pending.pop()
      if (visited.has(dependency)) continue
      visited.add(dependency)
      assert(Object.hasOwn(report.vulnerabilities, dependency), `Missing audit dependency: ${dependency}`)
      const finding = report.vulnerabilities[dependency]
      assert(Array.isArray(finding.via) && finding.via.length > 0, `Missing advisory evidence: ${dependency}`)
      for (const via of finding.via) {
        if (typeof via === 'string') {
          pending.push(via)
          continue
        }
        assert(via && ['info', 'low', 'moderate', 'high', 'critical'].includes(via.severity), 'Malformed advisory')
        if (['info', 'low', 'moderate'].includes(via.severity)) continue
        const expected = backports.get(dependency)
        assert(expected && via.name === dependency && via.dependency === dependency &&
          via.url === expected[0] && via.range === expected[1] && via.severity === 'high',
        `Unpatched advisory: ${dependency} ${via.url}`)
        assert.deepEqual(finding.nodes, [`node_modules/${dependency}`], `Unpatched dependency copy: ${dependency}`)
        accepted.add(via.url)
        covered = true
      }
    }
    assert(covered, `No backported advisory explains high severity finding: ${name}`)
  }
  return [...accepted].sort()
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const cwd = fileURLToPath(new URL('../', import.meta.url))
  const audit = spawnSync('npm', ['audit', '--audit-level=high', '--json'], {
    cwd, encoding: 'utf8', maxBuffer: 10 * 1024 * 1024,
  })
  // Keep the original report, including failures, for CI artifacts and future upstream rechecks.
  writeFileSync(new URL('../npm-audit.json', import.meta.url), audit.stdout ?? '')
  if (audit.stderr) process.stderr.write(audit.stderr)
  assert.ifError(audit.error)
  const accepted = evaluateAudit(JSON.parse(audit.stdout), audit.status)
  execFileSync(process.execPath, ['scripts/patch-dependencies.mjs', '--check'], { cwd, stdio: 'inherit' })
  execFileSync('npm', ['test', '--', 'tests/dependencySecurity.test.ts'], { cwd, stdio: 'inherit' })
  const summary = accepted.length
    ? `Verified local backports (issue #7):\n${accepted.map(url => `- ${url}`).join('\n')}\n\nNo unpatched high/critical advisories. Raw findings remain in npm-audit.json.\n`
    : 'No high/critical advisories.\n'
  console.log(summary)
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary)
}
