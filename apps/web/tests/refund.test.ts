import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { REFUND_MAX, buildRefundBody, isFrozen, pendingRefunds, refundAmountError, refundError, refundStatusKey } from '../app/utils/refund'
import { idemKeys } from '../app/utils/topup'
import { CASH_REFUND_STATUSES, DEBT_STATUSES, TOPUP_FILTERS, cashRefundActions, cashRefundPatch, debtActions, debtPatch, disputeActions, disputeBody, disputeOutcome, listRows } from '../app/utils/walletOps'
import { t } from './setup'

describe('refundAmountError（金額範圍與可退餘額邊界）', () => {
  it.each([['1', 1], ['500', 500], [' 500 ', 500], ['3000', 3000]] as const)('接受 %s（可退 %s 以上）', (v, ref) => expect(refundAmountError(v, ref)).toBeNull())
  it('剛好等於可退餘額可送，多 1 點不行', () => {
    expect(refundAmountError('3000', 3000)).toBeNull()
    expect(refundAmountError('3001', 3000)).toBe('me.refund.err.amountOver')
  })
  it('可退餘額為 0 時任何金額都不可送', () => expect(refundAmountError('1', 0)).toBe('me.refund.err.amountOver'))
  it.each(['0', '50001', '00000'])('範圍外 %s', v => expect(refundAmountError(v, 99_999)).toBe('me.refund.err.amountRange'))
  it('上限 50,000 剛好可送（餘額足夠時）', () => {
    expect(REFUND_MAX).toBe(50_000)
    expect(refundAmountError('50000', 60_000)).toBeNull()
  })
  it.each(['', 'abc', '1e3', '100.5', '-100', '1,000', ' '])('非整數 %s', v => expect(refundAmountError(v, 99_999)).toBe('me.refund.err.amountInt'))
})

describe('buildRefundBody / Idempotency-Key 重用', () => {
  it('只送 amount（數字）', () => expect(buildRefundBody(' 800 ')).toEqual({ amount: 800 }))
  const mk = () => { let n = 0; return idemKeys(() => `k${++n}`) }
  it('相同內容重試沿用同一把 key，金額改變才換新', () => {
    const k = mk()
    expect(k.keyFor(buildRefundBody('500'))).toBe('k1')
    expect(k.keyFor(buildRefundBody('500'))).toBe('k1')
    expect(k.keyFor(buildRefundBody('600'))).toBe('k2')
  })
  it('成功後 reset，下一次一定是新 key', () => {
    const k = mk(); k.keyFor(buildRefundBody('500')); k.reset()
    expect(k.keyFor(buildRefundBody('500'))).toBe('k2')
  })
})

describe('refundError（錯誤碼對應）', () => {
  const fmt = (n: number) => n.toLocaleString('en-US')
  const run = (e: any) => refundError(e, t, fmt)
  it('NOT_CASH_REFUNDABLE 顯示 refundable_points 並要求重讀餘額', () => {
    const r = run({ code: 'NOT_CASH_REFUNDABLE', status: 409, body: { refundable_points: 1200, balance: 5000 } })
    expect(r.msg).toContain('1,200'); expect(r.msg).toContain('禮物卡與贊助點數不可退現金'); expect(r.reloadWallet).toBe(true)
  })
  it('NOT_CASH_REFUNDABLE 缺 refundable_points 時當作 0，不顯示 NaN / undefined', () => {
    expect(run({ code: 'NOT_CASH_REFUNDABLE', body: {} }).msg).toMatch(/只有 0 點/)
  })
  it('INSUFFICIENT_POINTS 顯示目前餘額', () => expect(run({ code: 'INSUFFICIENT_POINTS', body: { balance: 80 } }).msg).toContain('80'))
  it('WALLET_FROZEN 要求隱藏表單並請聯絡客服', () => {
    const r = run({ code: 'WALLET_FROZEN', status: 403 })
    expect(r.frozen).toBe(true); expect(r.msg).toContain('已凍結'); expect(r.msg).toContain('客服')
  })
  it('429 顯示約幾分鐘，讀不到時不亂編', () => {
    expect(run({ code: 'RATE_LIMITED', body: { retry_after: 1800 } }).msg).toContain('30 分鐘')
    expect(run({ code: 'RATE_LIMITED', body: {} }).msg).not.toMatch(/分鐘/)
  })
  it('422 / 網路 / 5xx / 其他', () => {
    expect(run({ code: 'VALIDATION_FAILED', status: 422 }).msg).toContain('內容有誤')
    expect(run({ code: 'NETWORK', status: 0 }).msg).toContain('連線')
    expect(run({ status: 500 }).msg).toContain('暫時無法')
    expect(run({ status: 409, detail: '後端訊息' }).msg).toBe('後端訊息')
    expect(run({}).msg).toContain('失敗')
  })
})

describe('狀態顯示', () => {
  it('四種狀態都有文案；未知狀態不崩', () => {
    for (const s of CASH_REFUND_STATUSES) expect(t(refundStatusKey(s)!)).not.toBe(refundStatusKey(s))
    expect(t(refundStatusKey('requested')!)).toBe('待審核')
    expect(refundStatusKey('weird')).toBeNull()
  })
  it('requested / processing 算審核中', () => {
    expect(pendingRefunds([{ status: 'requested' }, { status: 'processing' }, { status: 'completed' }, { status: 'rejected' }])).toBe(2)
    expect(pendingRefunds([])).toBe(0)
  })
  it('凍結：沒有 status 欄位視為正常', () => {
    expect(isFrozen({ status: 'frozen' })).toBe(true)
    expect(isFrozen({ status: 'active' })).toBe(false)
    expect(isFrozen({})).toBe(false); expect(isFrozen(null)).toBe(false)
  })
})

describe('後台動作對應', () => {
  it('現金退款狀態機：requested → processing | rejected；processing → completed | rejected；其餘無動作', () => {
    expect(cashRefundActions('requested')).toEqual(['processing', 'rejected'])
    expect(cashRefundActions('processing')).toEqual(['completed', 'rejected'])
    expect(cashRefundActions('completed')).toEqual([]); expect(cashRefundActions('rejected')).toEqual([])
  })
  it('現金退款 PATCH：rejected 理由必填（空白也不行）並送 reject_reason；其餘不送理由', () => {
    expect(cashRefundPatch('rejected', '  ')).toEqual({ error: 'admin.cashRefunds.reasonRequired' })
    expect(cashRefundPatch('rejected', ' 帳戶資料不符 ')).toEqual({ body: { status: 'rejected', reject_reason: '帳戶資料不符' } })
    expect(cashRefundPatch('completed', 'ignored')).toEqual({ body: { status: 'completed' } })
    expect(cashRefundPatch('processing', '')).toEqual({ body: { status: 'processing' } })
  })
  it('欠款：只有 open 可結案；note 必填', () => {
    expect(debtActions('open')).toEqual(['settled', 'written_off'])
    expect(debtActions('settled')).toEqual([]); expect(debtActions('written_off')).toEqual([])
    expect(debtPatch('written_off', '')).toEqual({ error: 'admin.walletDebts.noteRequired' })
    expect(debtPatch('settled', ' 已追回 ')).toEqual({ body: { status: 'settled', note: '已追回' } })
  })
  it('爭議：paid → open；disputed → won | lost；其餘無動作', () => {
    expect(disputeActions('paid')).toEqual(['open'])
    expect(disputeActions('disputed')).toEqual(['won', 'lost'])
    for (const s of ['charged_back', 'pending', 'failed', 'expired', 'refunded']) expect(disputeActions(s)).toEqual([])
  })
  it('爭議 body：action 必送，note 選填（空白不送）', () => {
    expect(disputeBody('open', '')).toEqual({ action: 'open' })
    expect(disputeBody('lost', ' 金流商判定 ')).toEqual({ action: 'lost', note: '金流商判定' })
  })
  it('爭議後的列狀態：open / lost 會凍結錢包；won 是否解凍未知', () => {
    expect(disputeOutcome('open')).toEqual({ status: 'disputed', walletFrozen: true })
    expect(disputeOutcome('lost')).toEqual({ status: 'charged_back', walletFrozen: true })
    expect(disputeOutcome('won')).toEqual({ status: 'paid', walletFrozen: null })
  })
  it('列表容忍 data / topups 欄位名', () => {
    expect(listRows({ data: [1] })).toEqual([1]); expect(listRows({ topups: [2] })).toEqual([2]); expect(listRows({})).toEqual([])
  })
  it('篩選與狀態常數', () => {
    expect(TOPUP_FILTERS[0]).toBe('disputed'); expect(TOPUP_FILTERS).toContain('paid')
    expect([...DEBT_STATUSES]).toEqual(['open', 'settled', 'written_off'])
  })
})

// 三語系文案完整性：key 集合一致、插值參數一致，且頁面用到的靜態 key 都存在
describe('現金退款 i18n 完整性', () => {
  const root = `${process.cwd()}/i18n/locales`
  const load = (loc: string, f: string) => JSON.parse(readFileSync(`${root}/${loc}/${f}.json`, 'utf8'))
  const locales = ['zh-Hant', 'en', 'vi']
  const flat = (o: any, p = ''): Record<string, string> =>
    Object.fromEntries(Object.entries(o).flatMap(([k, v]) => typeof v === 'string' ? [[p + k, v]] : Object.entries(flat(v, `${p}${k}.`))))
  const all = (loc: string) => ({ ...flat(load(loc, 'me')), ...flat(load(loc, 'admin')) })
  const scoped = (loc: string) => Object.fromEntries(Object.entries(all(loc)).filter(([k]) => /^(me\.refund\.|me\.frozenBanner|me\.wallet\.refund|admin\.(cashRefunds|walletDebts|topups)\.|admin\.index\.nav\.(cashRefunds|walletDebts|topups))/.test(k)))
  const params = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort()

  it('en / vi 與 zh-Hant 的 key 集合相同', () => {
    const base = Object.keys(scoped('zh-Hant')).sort()
    expect(base.length).toBeGreaterThan(100)
    for (const loc of locales.slice(1)) expect(Object.keys(scoped(loc)).sort(), loc).toEqual(base)
  })
  it('每個 key 的插值參數在三語系一致，且沒有空字串', () => {
    const base = scoped('zh-Hant')
    for (const loc of locales) for (const [k, v] of Object.entries(scoped(loc))) {
      expect(v.trim(), `${loc} ${k}`).not.toBe('')
      expect(params(v), `${loc} ${k}`).toEqual(params(base[k]!))
    }
  })
  it('頁面與工具程式用到的靜態 key 都有翻譯', () => {
    const files = ['app/pages/me/refund.vue', 'app/pages/me/wallet.vue', 'app/pages/me/topup.vue', 'app/pages/admin/cash-refunds.vue', 'app/pages/admin/wallet-debts.vue', 'app/pages/admin/topups.vue', 'app/utils/refund.ts', 'app/utils/walletOps.ts']
    const used = new Set<string>()
    for (const f of files) for (const m of readFileSync(`${process.cwd()}/${f}`, 'utf8').matchAll(/['"`]((?:me\.refund|me\.frozenBanner|me\.wallet\.refund\w*|admin\.(?:cashRefunds|walletDebts|topups)|admin\.index\.nav)\.[\w.]+)['"`]/g)) used.add(m[1]!)
    expect(used.size).toBeGreaterThan(20)
    for (const loc of locales) for (const k of used) expect(all(loc)[k], `${loc} ${k}`).toBeTypeOf('string')
  })
  it('動態 key（狀態 / 動作 / 確認文案）逐一存在', () => {
    for (const loc of locales) {
      const m = all(loc)
      for (const s of CASH_REFUND_STATUSES) for (const p of ['me.refund.status', 'admin.cashRefunds.status']) expect(m[`${p}.${s}`], `${loc} ${p}.${s}`).toBeTypeOf('string')
      for (const a of ['processing', 'completed', 'rejected']) for (const p of ['action', 'confirm', 'done']) expect(m[`admin.cashRefunds.${p}.${a}`], `${loc} ${p}.${a}`).toBeTypeOf('string')
      for (const a of ['settled', 'written_off']) for (const p of ['action', 'confirm', 'done']) expect(m[`admin.walletDebts.${p}.${a}`], `${loc} ${p}.${a}`).toBeTypeOf('string')
      for (const s of DEBT_STATUSES) expect(m[`admin.walletDebts.status.${s}`]).toBeTypeOf('string')
      for (const a of ['open', 'won', 'lost']) for (const p of ['action', 'confirm', 'done']) expect(m[`admin.topups.${p}.${a}`], `${loc} ${p}.${a}`).toBeTypeOf('string')
      for (const s of TOPUP_FILTERS.filter(Boolean)) expect(m[`admin.topups.status.${s}`]).toBeTypeOf('string')
    }
  })
})
