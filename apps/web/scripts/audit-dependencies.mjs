import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'

// 唯一的限期例外（見 patches/README.md）。不做成設定檔；到期後自動失效，須升版後刪除此筆。
export const EXEMPTIONS = [{
  ghsa: 'GHSA-fc5q-39xc-vjpf',
  reason: 'picomatch 上游尚無修補版；只在建置時比對專案內 glob、不處理外部輸入',
  expires: '2026-12-18',
}]

const isGhsa = (v, ghsa) => typeof v === 'object' && v !== null
  && (v.url?.toLowerCase().endsWith(`/${ghsa.toLowerCase()}`) || v.ghsa?.toLowerCase() === ghsa.toLowerCase())

// 回傳 { exempt, packages, entry }。npm 原本失敗且所有 high/critical 都能由該公告（含遞移）解釋才豁免。
// 「可豁免」以最大不動點計算：先假設全部可豁免，反覆剔除有其他公告或指向不可豁免套件的項目，循環引用因此不會無限迴圈。
export function auditExemption(audit, today = new Date().toISOString().slice(0, 10), entries = EXEMPTIONS) {
  const none = { exempt: false, packages: 0, entry: null }
  const vulns = audit?.vulnerabilities
  if (!vulns || typeof vulns !== 'object') return none
  const serious = Object.keys(vulns).filter(n => ['high', 'critical'].includes(vulns[n]?.severity))
  if (!serious.length) return none
  for (const entry of entries) {
    if (today > entry.expires) continue
    const ok = new Set(serious.filter(n => Array.isArray(vulns[n]?.via) && vulns[n].via.length))
    const minor = v => vulns[v] && !serious.includes(v) // moderate 以下不影響結果，視為已滿足
    for (let changed = true; changed;) {
      changed = false
      for (const n of [...ok]) {
        const good = vulns[n].via.every(v => typeof v === 'string' ? ok.has(v) || minor(v) : isGhsa(v, entry.ghsa))
        if (!good) { ok.delete(n); changed = true }
      }
    }
    // 純循環、沒有任何該公告支撐的套件不算：從帶有該公告的套件沿相依方向傳遞「有根據」。
    const grounded = new Set([...ok].filter(n => vulns[n].via.some(v => typeof v === 'object' && isGhsa(v, entry.ghsa))))
    for (let changed = true; changed;) {
      changed = false
      for (const n of ok) {
        if (!grounded.has(n) && vulns[n].via.some(v => grounded.has(v))) { grounded.add(n); changed = true }
      }
    }
    if (serious.every(n => grounded.has(n))) return { exempt: true, packages: serious.length, entry }
  }
  return none
}

function main() {
  const audit = spawnSync('npm', ['audit', '--audit-level=high', '--json'], {
    cwd: fileURLToPath(new URL('../', import.meta.url)),
    encoding: 'utf8', maxBuffer: 10 * 1024 * 1024,
  })
  // npm-audit.json 一律寫入 npm 的原始輸出；只有 npm 失敗、輸出可解析且符合 EXEMPTIONS 時才把結束碼改為 0。
  writeFileSync(new URL('../npm-audit.json', import.meta.url), audit.stdout ?? '')
  if (audit.stdout) process.stdout.write(audit.stdout)
  if (audit.stderr) process.stderr.write(audit.stderr)
  assert.ifError(audit.error)
  process.exitCode = audit.status ?? 1
  if (process.exitCode === 0) return
  let parsed
  try { parsed = JSON.parse(audit.stdout) } catch { return }
  const { exempt, packages, entry } = auditExemption(parsed)
  if (exempt) {
    process.stderr.write(`audit: 豁免 ${entry.ghsa}（影響 ${packages} 個套件，${entry.expires} 到期）：${entry.reason}\n`)
    process.exitCode = 0
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main()
