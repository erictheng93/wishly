// 站內通知串流是長連線，開著就永遠等不到 networkidle。回 200 空串流：前端照常抓未讀數後每幾秒重連，
// 不像 401 會在 console 留錯誤、混進腳本蒐集的 problems。真正的串流由 e2e/inbox-realtime.spec.ts 涵蓋。
export const shortenInboxStream = ctx => ctx.route('**/me/notifications/stream', r => r.fulfill({
  status: 200,
  contentType: 'text/event-stream',
  headers: { 'access-control-allow-origin': r.request().headers().origin ?? '*', 'access-control-allow-credentials': 'true' },
  body: '',
}))
