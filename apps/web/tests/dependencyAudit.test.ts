import { spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { delimiter, join } from 'node:path'
import { describe, expect, it } from 'vitest'

const bracesUrl = 'https://github.com/advisories/GHSA-vfj7-8cjw-p6xm'
const forgeUrl = 'https://github.com/advisories/GHSA-86w9-cpqp-85rv'
const advisory = (name: string, url: string, range: string) => ({
  name, dependency: name, severity: 'high', url, range,
})
const vulnerability = (name: string, via: any[], severity = 'high') => ({
  name, severity, via, nodes: [`node_modules/${name}`],
})
const report = (vulnerabilities: Record<string, any>) => ({
  auditReportVersion: 2,
  vulnerabilities,
  metadata: { vulnerabilities: {
    high: Object.values(vulnerabilities).filter(v => v.severity === 'high').length,
    critical: Object.values(vulnerabilities).filter(v => v.severity === 'critical').length,
  } },
})

// The same eleven entries and cyclic Nuxt dependency paths as the failing CI artifact.
const knownReport = () => report(Object.fromEntries([
  ['braces', [advisory('braces', bracesUrl, '<=3.0.3')]],
  ['node-forge', [advisory('node-forge', forgeUrl, '<=1.4.0')]],
  ['micromatch', ['braces']], ['fast-glob', ['micromatch']],
  ['globby', ['fast-glob', 'micromatch']], ['listhen', ['node-forge']],
  ['nitropack', ['listhen']], ['@nuxt/nitro-server', ['nitropack', 'nuxt']],
  ['@nuxt/vite-builder', ['nuxt']], ['@nuxt/vite-server', ['nuxt']],
  ['nuxt', ['@nuxt/nitro-server', '@nuxt/vite-builder', '@nuxt/vite-server']],
].map(([name, via]) => [name, vulnerability(name as string, via as any[])])))

describe('raw npm audit signal', () => {
  it.each([
    ['clean audit', report({}), 0],
    ['the two locally backported advisories and their dependency chains', knownReport(), 1],
    ['scanner failure', { error: { code: 'ENOAUDIT' } }, 2],
  ])('preserves exit status and unfiltered JSON for %s', (_name, current, status) => {
    const directory = mkdtempSync(join(tmpdir(), 'wishly-audit-'))
    const output = JSON.stringify(current, null, 2) + '\n'
    try {
      mkdirSync(join(directory, 'scripts'))
      mkdirSync(join(directory, 'bin'))
      copyFileSync(new URL('../scripts/audit-dependencies.mjs', import.meta.url), join(directory, 'scripts/audit-dependencies.mjs'))
      writeFileSync(join(directory, 'bin/npm'), `#!/usr/bin/env node
if (process.argv[2] === 'audit') {
  process.stdout.write(${JSON.stringify(output)});
  process.exitCode = ${status};
}
`, { mode: 0o755 })
      const result = spawnSync(process.execPath, ['scripts/audit-dependencies.mjs'], {
        cwd: directory, encoding: 'utf8',
        env: { ...process.env, PATH: join(directory, 'bin') + delimiter + process.env.PATH },
      })
      expect(result.status, result.stderr).toBe(status)
      expect(readFileSync(join(directory, 'npm-audit.json'), 'utf8')).toBe(output)
    } finally {
      rmSync(directory, { recursive: true, force: true })
    }
  })
})
