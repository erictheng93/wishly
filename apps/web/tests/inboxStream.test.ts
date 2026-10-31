import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { pickToast, startInboxStream } from '../app/utils/inboxStream'

const enc = new TextEncoder()
// 可手動推 chunk 的假串流
function conn(status = 200) {
  let c!: ReadableStreamDefaultController<Uint8Array>
  const body = new ReadableStream<Uint8Array>({ start(x) { c = x } })
  return { res: new Response(body, { status }), push: (s: string) => c.enqueue(enc.encode(s)), end: () => c.close(), fail: () => c.error(new Error('aborted')) }
}
const tick = (ms = 0) => vi.advanceTimersByTimeAsync(ms)

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

function setup(conns: Array<ReturnType<typeof conn> | 'fail'>, extra = {}) {
  const onEvent = vi.fn(); let n = 0
  const fetchImpl = vi.fn(async () => { const c = conns[Math.min(n++, conns.length - 1)]!; if (c === 'fail') throw new Error('net'); return c.res }) as any
  const s = startInboxStream({ url: 'u', headers: () => ({}), onEvent, fetchImpl, ...extra })
  return { s, onEvent, fetchImpl }
}

describe('startInboxStream', () => {
  it('連上補一次，之後每個 inbox 事件觸發；其他事件忽略', async () => {
    const c = conn(); const { s, onEvent } = setup([c])
    await tick(); expect(onEvent).toHaveBeenCalledTimes(1)
    c.push('retry: 3000\n\n'); c.push('event: ping\ndata: {}\n\n'); await tick(); expect(onEvent).toHaveBeenCalledTimes(1)
    c.push('event: inbox\ndata: {}\n\n'); await tick(); expect(onEvent).toHaveBeenCalledTimes(2)
    s.stop()
  })
  it('事件被切在兩個 chunk 也能解析（含 CRLF）', async () => {
    const c = conn(); const { s, onEvent } = setup([c])
    await tick()
    c.push('event: inb'); await tick(); c.push('ox\r\ndata: {}\r'); await tick(); expect(onEvent).toHaveBeenCalledTimes(1)
    c.push('\n\r\n'); await tick(); expect(onEvent).toHaveBeenCalledTimes(2)
    s.stop()
  })
  it('斷線後依退避重連並再補一次', async () => {
    const a = conn(), b = conn(); const { s, onEvent, fetchImpl } = setup([a, b])
    await tick(); a.end(); await tick(1999); expect(fetchImpl).toHaveBeenCalledTimes(1)
    await tick(1); expect(fetchImpl).toHaveBeenCalledTimes(2); expect(onEvent).toHaveBeenCalledTimes(2)
    s.stop()
  })
  it('401 / 403 停止，不重試', async () => {
    const { onEvent, fetchImpl } = setup([conn(401)])
    await tick(60000); expect(fetchImpl).toHaveBeenCalledTimes(1); expect(onEvent).not.toHaveBeenCalled()
    const r = setup([conn(403)]); await tick(60000); expect(r.fetchImpl).toHaveBeenCalledTimes(1)
  })
  it('連續失敗 3 次起每 15 秒輪詢，連上後停止', async () => {
    const ok = conn(); const { s, onEvent } = setup(['fail', 'fail', 'fail', ok], { delays: [1000, 1000, 40000] })
    await tick(); await tick(1000); await tick(1000) // 第 3 次失敗，開始輪詢，下次重連在 40 秒後
    expect(onEvent).not.toHaveBeenCalled()
    await tick(15000); expect(onEvent).toHaveBeenCalledTimes(1)
    await tick(15000); expect(onEvent).toHaveBeenCalledTimes(2)
    await tick(10000) // 連上：補一次並停止輪詢
    expect(onEvent).toHaveBeenCalledTimes(3)
    await tick(60000); expect(onEvent).toHaveBeenCalledTimes(3)
    s.stop()
  })
  it('連線無聲卡死時看門狗中止並重連；心跳會重置看門狗', async () => {
    const c = conn(), c2 = conn()
    let n = 0
    const fetchImpl = vi.fn(async (_: string, i: RequestInit) => { const x = n++ ? c2 : c; i.signal!.addEventListener('abort', x.fail); return x.res }) as any
    const s = startInboxStream({ url: 'u', headers: () => ({}), onEvent: vi.fn(), fetchImpl })
    await tick(); await tick(40000); c.push(': heartbeat\n\n'); await tick(40000)
    expect(fetchImpl).toHaveBeenCalledTimes(1)
    await tick(5000 + 2000); expect(fetchImpl).toHaveBeenCalledTimes(2)
    s.stop()
  })
  it('stop() 中止連線且不再重連', async () => {
    const c = conn(); const { s, fetchImpl } = setup([c])
    await tick(); s.stop(); await tick(60000); expect(fetchImpl).toHaveBeenCalledTimes(1)
  })
})

describe('pickToast', () => {
  const n = (id: string, read = false) => ({ id, read_at: read ? 'x' : null })
  it('首次載入不跳', () => expect(pickToast(null, [n('a')], 1)).toBeNull())
  it('新的未讀才跳', () => {
    expect(pickToast({ top: 'a', unread: 1 }, [n('b'), n('a')], 2)?.id).toBe('b')
    expect(pickToast({ top: 'a', unread: 1 }, [n('a')], 1)).toBeNull()
    expect(pickToast({ top: 'a', unread: 2 }, [n('b', true)], 1)).toBeNull()
  })
})
