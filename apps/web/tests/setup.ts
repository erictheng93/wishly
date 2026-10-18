// 測試環境沒有 Nuxt：用 zh-Hant 語系檔提供 t / $t / useI18n / useNuxtApp().$i18n（直接賦值，不受 vi.unstubAllGlobals 影響）
import { readdirSync, readFileSync } from 'node:fs'
import { ref } from 'vue'

const dir = `${process.cwd()}/i18n/locales/zh-Hant/`
const messages: Record<string, any> = {}
for (const f of readdirSync(dir)) Object.assign(messages, JSON.parse(readFileSync(dir + f, 'utf8')))

export const t = (key: string, params: Record<string, any> = {}): string => {
  const v = key.split('.').reduce((o: any, k) => o?.[k], messages)
  return typeof v === 'string' ? v.replace(/\{(\w+)\}/g, (_, k) => String(params[k] ?? `{${k}}`)) : key
}
const i18n = { t, locale: ref('zh-Hant') }
Object.assign(globalThis, { useI18n: () => i18n, useNuxtApp: () => ({ $i18n: i18n }) })
