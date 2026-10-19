import type { Page, Route } from '@playwright/test'
import { WEB } from './env'

// 以 page.route 模擬後端端點的共用小工具：登入走真後端，只有被測的錢包 / 後台端點是假的。
export const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
export const iso = (ms = 0) => new Date(Date.now() + ms).toISOString()
export const cors = { 'access-control-allow-origin': WEB, 'access-control-allow-credentials': 'true', 'access-control-expose-headers': 'retry-after' }

export type Res = { status: number, json: any, headers?: Record<string, string> }
export const ok = (json: any, status = 200): Res => ({ status, json })
export const problem = (status: number, code: string, detail: string, extra: object = {}): Res => ({ status, json: { type: 'about:blank', title: code, status, code, detail, ...extra } })

export function reply(route: Route, r: Res) {
  return route.fulfill({ status: r.status, headers: { ...cors, ...r.headers }, contentType: r.status >= 400 ? 'application/problem+json' : 'application/json', body: JSON.stringify(r.json) })
}

export interface Call { method: string, url: URL, headers: Record<string, string>, body: any }
/** 攔截符合 pattern 的請求；預檢（OPTIONS）直接放行並回 CORS 標頭；每次呼叫記錄在回傳的陣列 */
export async function mock(page: Page, pattern: RegExp, handler: (c: Call, n: number) => Res | Promise<Res>): Promise<Call[]> {
  const calls: Call[] = []
  await page.route(pattern, async (route) => {
    const req = route.request()
    if (req.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { ...cors, 'access-control-allow-headers': 'content-type,idempotency-key,accept-language,x-native-client,authorization', 'access-control-allow-methods': 'GET,POST,PATCH,PUT,DELETE,OPTIONS' } })
    const c: Call = { method: req.method(), url: new URL(req.url()), headers: req.headers(), body: req.method() === 'GET' ? null : (req.postDataJSON() ?? null) }
    calls.push(c)
    return reply(route, await handler(c, calls.length))
  })
  return calls
}
