import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { createI18n } from 'vue-i18n'

// 英文區間句的 1 天／多天兩個分支各自是完整句子（| 會切開整則訊息，不是只切括號內）
const en = JSON.parse(readFileSync(fileURLToPath(new URL('../i18n/locales/en/admin.json', import.meta.url)), 'utf8')).admin
const i18n = createI18n({ legacy: false, locale: 'en', messages: { en: { admin: en } } })
const t = i18n.global.t as (k: string, n: object, plural: number) => string

describe('en range 單複數', () => {
  for (const key of ['admin.finance.range', 'admin.analytics.range']) {
    it(key, () => {
      const p = { from: '2026-10-01', to: '2026-10-31' }
      expect(t(key, { ...p, days: 1 }, 1)).toBe('Range: 2026-10-01 – 2026-10-31 (1 day)')
      expect(t(key, { ...p, days: 30 }, 30)).toBe('Range: 2026-10-01 – 2026-10-31 (30 days)')
    })
  }
})
