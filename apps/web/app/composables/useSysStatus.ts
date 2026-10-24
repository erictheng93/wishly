// 全站系統狀態（高負載預警橫幅）：API 暫時不可用就記下，Retry-After 到期自動消失；唯讀維護另由 /public/status 預先得知
export type SysStatus = { kind: 'busy' | 'readOnly', until: number } | null
export const useSysStatus = () => useState<SysStatus>('sys-status', () => null)

// 503 是後端自己回的；502/504/52x/530 是 Cloudflare 在源站當掉、逾時或 Tunnel 斷線時產生的（SSR 讀得到）
const DOWN = new Set([502, 503, 504, 520, 521, 522, 523, 524, 525, 526, 527, 530])

// status 0 = 連不到：瀏覽器讀不到 Cloudflare 錯誤頁（沒有 CORS 標頭）也會是 0；使用者自己離線時不算
export function noteUnavailable(status: number, code?: string, retryAfter?: number) {
  if (!DOWN.has(status) && !(status === 0 && globalThis.navigator?.onLine !== false)) return
  useSysStatus().value = { kind: code === 'READ_ONLY_MODE' ? 'readOnly' : 'busy', until: Date.now() + (retryAfter || 30) * 1000 }
}
