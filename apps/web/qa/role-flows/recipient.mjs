// 角色流程模擬：用戶・受捐者（開心願清單的人）。設計與結果見 docs/role-flows/01-user-recipient.md
// 執行：cd apps/web && node qa/role-flows/recipient.mjs   → qa/out/role-flows/recipient.json 與 recipient/*.png
// 原則：能用 UI 就用 UI；只有「平台外」動作（取 OTP、時間快轉改 DB、製造另一位用戶、營運推進採購單）走 API/SQL，並在 note 註明。
import { expect } from '@playwright/test'
import { readFileSync, writeFileSync } from 'node:fs'
import { deflateSync } from 'node:zlib'
import { resolve } from 'node:path'
import {
  API, Blocked, Gap, OUT, PASSWORD, WEB, apiCtx, closeBrowser, listMail, newPage, otpFrom, q, randomUUID, recorder,
  registerApi, randomSecret, resetOtpLimits, sleep, sql, uid, uiLogin, waitForMail,
} from './lib.mjs'

const run = recorder('recipient')
const tag = uid()
const S = {} // 跨步驟共用狀態
const need = (...ks) => { for (const k of ks) if (!S[k]) throw new Error(`前置步驟未完成：缺 ${k}`) }
const hydrated = p => p.waitForFunction(() => window.useNuxtApp?.().isHydrating === false, null, { timeout: 15000 }).catch(() => p.waitForLoadState('networkidle'))
const inDays = d => new Date(Date.now() + d * 864e5 + 8 * 36e5).toISOString() // 台北時間偏移後取 slice
const dateIn = d => inDays(d).slice(0, 10)
const localIn = d => inDays(d).slice(0, 16)
const noHScroll = p => p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)

// ---------- 測試用 PNG（純 node，64x64 實色） ----------
function crc32(buf) { let c, crc = ~0; for (const b of buf) { c = (crc ^ b) & 0xff; for (let k = 0; k < 8; k++) c = c & 1 ? (c >>> 1) ^ 0xedb88320 : c >>> 1; crc = (crc >>> 8) ^ c } return ~crc >>> 0 }
function png(path, [r, g, b]) {
  const w = 64, h = 64, raw = Buffer.alloc((w * 3 + 1) * h)
  for (let y = 0; y < h; y++) { raw[y * (w * 3 + 1)] = 0; for (let x = 0; x < w; x++) { const o = y * (w * 3 + 1) + 1 + x * 3; raw[o] = r; raw[o + 1] = g; raw[o + 2] = b } }
  const chunk = (t, d) => { const len = Buffer.alloc(4); len.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc32(td)); return Buffer.concat([len, td, c]) }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2
  writeFileSync(path, Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]))
}
const PNG_ITEM = resolve(OUT, 'recipient', 'item.png'), PNG_COVER = resolve(OUT, 'recipient', 'cover.png')
png(PNG_ITEM, [200, 16, 46]); png(PNG_COVER, [30, 90, 160])

// ---------- UI 操作封裝 ----------
async function uiRegister(page, { name, email, password = PASSWORD }) {
  resetOtpLimits()
  await page.goto('/register'); await hydrated(page)
  await page.getByLabel('顯示名稱').fill(name)
  await page.getByLabel('Email').fill(email)
  await page.getByLabel('密碼', { exact: true }).fill(password)
  const t0 = Date.now() - 1000
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  await expect(page.getByText(`驗證碼已寄到 ${email}`)).toBeVisible()
  return t0
}
async function uiLogout(page) {
  await page.goto('/settings'); await hydrated(page)
  await page.getByRole('button', { name: '登出', exact: true }).click()
  await page.waitForURL(/\/login/)
}
/** 新增清單頁；回傳清單 id。o: title,desc,event,closes,surprise,showNames,vis,cover,pick(摯友名稱陣列),ttl,bday */
async function uiCreateList(page, o) {
  await page.goto('/lists/new'); await hydrated(page)
  await page.getByLabel(/清單名稱/).fill(o.title)
  if (o.desc) await page.getByLabel('說明（選填）').fill(o.desc)
  if (o.cover) await page.getByRole('radio', { name: new RegExp(o.cover) }).check({ force: true })
  if (o.event) await page.getByLabel('活動日（選填）').fill(o.event)
  if (o.closes) await page.getByLabel(/自動結束時間/).fill(o.closes)
  if (o.ttl) await page.getByLabel('購買期限').selectOption({ label: o.ttl })
  if (o.surprise) await page.getByLabel('驚喜模式').check()
  if (o.showNames) await page.getByLabel('顯示送禮者暱稱給其他訪客').check()
  if (o.vis) await page.getByRole('radio', { name: new RegExp('^' + o.vis) }).check()
  for (const n of o.pick ?? []) await page.getByLabel(n).check()
  if (o.bday) await page.getByLabel(/生日前 7 天通知/).check()
  await page.getByRole('button', { name: '建立並新增品項' }).click()
  await page.waitForURL(/\/lists\/[0-9a-f-]+\/edit/)
  await hydrated(page)
  return page.url().match(/lists\/([0-9a-f-]+)\/edit/)[1]
}
/** 在編輯頁新增品項。o: title,brand,spec,url,qty,price,prio(高/中/低),crowd{target,deadline,ref},image */
async function uiAddItem(page, o) {
  await page.getByRole('button', { name: '＋ 新增品項' }).click()
  const d = page.getByRole('dialog', { name: '新增品項' })
  await d.getByLabel('名稱（必填）').fill(o.title)
  if (o.brand) await d.getByLabel('品牌（選填）').fill(o.brand)
  if (o.spec) await d.getByLabel('規格（選填）').fill(o.spec)
  if (o.url) await d.getByLabel('商品網址（選填）').fill(o.url)
  if (o.crowd) {
    await d.getByRole('radio', { name: '心願眾籌' }).check()
    await d.getByLabel(/目標點數/).fill(String(o.crowd.target))
    if (o.crowd.deadline) await d.getByLabel(/募集截止/).fill(o.crowd.deadline)
    if (o.crowd.ref) await d.getByLabel(/商品參考價格/).fill(String(o.crowd.ref))
  } else {
    if (o.qty) await d.getByLabel(/需要數量/).fill(String(o.qty))
    if (o.price) await d.getByLabel(/參考價格/).fill(String(o.price))
  }
  if (o.prio) await d.getByRole('radio', { name: o.prio, exact: true }).check()
  if (o.image) { await d.locator('input[type=file]').setInputFiles(o.image); await expect(d.getByText(/已上傳|上傳失敗|圖片/).first()).toBeVisible({ timeout: 15000 }) }
  return d
}
const saveItem = async (page, d) => { await d.getByRole('button', { name: '儲存品項' }).click() }
const itemCard = (page, title) => page.locator('article.c-card', { hasText: title })
async function publish(page) {
  await page.getByRole('button', { name: '發佈並分享' }).click()
  const dlg = page.getByRole('dialog', { name: '分享你的清單' })
  await expect(dlg).toBeVisible()
  const url = await dlg.locator('#c-share-input').inputValue()
  return { dlg, url, slug: url.split('/').pop().split('?')[0] }
}
const ownerLogin = async (email, password = PASSWORD) => { const a = await apiCtx(); const r = await a.post('auth/login', { data: { email, password } }); if (!r.ok()) throw new Error('api login ' + r.status()); return a }
const meId = async a => { const j = await (await a.get('me')).json(); return j.user?.id ?? j.id }
const claimApi = async (u, itemId, body = { qty: 1 }) => { const r = await u.api.post(`items/${itemId}/claims`, { data: body, headers: { 'Idempotency-Key': randomUUID() } }); if (!r.ok()) throw new Error(`claim ${r.status()} ${await r.text()}`); return r.json() }
const wl = async (a, id) => (await (await a.get(`wishlists/${id}`)).json())
const closeSheet = async dlg => { await dlg.getByRole('button', { name: '關閉' }).last().click() }

// ===================================================================================
console.log(`recipient 測試批次 ${tag}`)
const owner = { name: `受捐者阿美${tag.slice(0, 3)}`, email: `flow-recipient-${tag}@example.com`, password: PASSWORD }
S.owner = owner
const page = await newPage()
await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], { origin: WEB }).catch(() => {})

// ---------- A. 帳號建立與首次登入 ----------
await run.step('R-01', '未登入首頁：看得到登入／建立帳號入口，受限頁被導回登入', async ({ note }) => {
  await page.goto('/'); await hydrated(page)
  const links = await page.getByRole('link').allInnerTexts()
  note('首頁連結：' + links.map(s => s.trim()).filter(Boolean).slice(0, 12).join(' | '))
  await expect(page.getByRole('link', { name: /登入|開始|建立/ }).first()).toBeVisible()
  await page.goto('/dashboard')
  await expect(page).toHaveURL(/\/login\?redirect=(%2F|\/)dashboard/)
  note('未登入進 /dashboard → 導向 ' + page.url())
  await page.goto('/lists/new'); await expect(page).toHaveURL(/\/login/)
  await page.goto('/settings'); await expect(page).toHaveURL(/\/login/)
  note('/lists/new、/settings 同樣導向登入')
}, { page })

await run.step('R-02', '註冊（UI）：密碼太短、錯誤 OTP、正確 OTP → 自動進 dashboard', async ({ note }) => {
  await page.goto('/register'); await hydrated(page)
  await page.getByLabel('顯示名稱').fill(owner.name)
  await page.getByLabel('Email').fill(owner.email)
  await page.getByLabel('密碼', { exact: true }).fill('short')
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  await expect(page.getByRole('alert')).toHaveText('密碼至少需 8 個字元')
  resetOtpLimits(); note('平台外：清除每 IP OTP 限流（多角色共用同 IP）')
  await page.getByLabel('密碼', { exact: true }).fill(owner.password)
  const t0 = Date.now() - 1000
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  await expect(page.getByText(`驗證碼已寄到 ${owner.email}`)).toBeVisible()
  await page.getByLabel('第 1 碼').fill('000000')
  await expect(page.getByRole('alert')).toContainText('驗證碼不正確或已過期')
  const m = await waitForMail(owner.email, { after: t0 })
  note(`平台外：從 Mailpit 取 OTP（主旨「${m.subject}」）`)
  await page.getByLabel('第 1 碼').fill(otpFrom(m.text))
  await expect(page).toHaveURL(/\/dashboard$/, { timeout: 15000 })
  await hydrated(page)
}, { page })

await run.step('R-03', '首次登入導覽：dashboard 空狀態引導、頁首選單、側欄', async ({ note }) => {
  await expect(page.getByText('還沒有清單，建立第一份吧')).toBeVisible()
  await expect(page.getByRole('link', { name: '＋ 建立清單' })).toBeVisible()
  const welcome = await page.getByText(/歡迎|導覽|教學|開始使用/).count()
  note(`空狀態 CTA「＋ 建立清單」可見；歡迎／導覽／教學文字數量 = ${welcome}`)
  await page.getByRole('button', { name: /帳號選單/ }).click()
  const menu = await page.getByRole('navigation', { name: '帳號選單' }).innerText()
  note('帳號選單：' + menu.replace(/\n+/g, ' | '))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-03-menu.png') })
  await page.keyboard.press('Escape')
  if (!welcome) { S.noTour = true }
}, { page })

await run.step('R-04', '重複註冊同一 Email：不洩漏帳號存在性，驗證後才提示', async ({ note }) => {
  const p2 = await newPage()
  try {
    resetOtpLimits()
    await p2.goto('/register'); await hydrated(p2)
    await p2.getByLabel('顯示名稱').fill('重複者')
    await p2.getByLabel('Email').fill(owner.email)
    await p2.getByLabel('密碼', { exact: true }).fill(randomSecret())
    const t0 = Date.now() - 1000
    await p2.getByRole('button', { name: '寄送驗證碼' }).click()
    await expect(p2.getByText(`驗證碼已寄到 ${owner.email}`)).toBeVisible()
    note('已存在的 Email 仍進到 OTP 步驟（不即時洩漏）')
    let m
    try { m = await waitForMail(owner.email, { after: t0, timeout: 6000 }) } catch { note('已註冊 Email 沒有收到第二封註冊 OTP') }
    if (m) {
      note(`收到信：「${m.subject}」`)
      await p2.getByLabel('第 1 碼').fill(otpFrom(m.text))
      await expect(p2.getByRole('alert')).toContainText('已註冊', { timeout: 10000 })
      note('輸入 OTP 後顯示：' + await p2.getByRole('alert').innerText())
    } else {
      await p2.getByLabel('第 1 碼').fill('123456')
      note('亂填 OTP 後訊息：' + (await p2.getByRole('alert').innerText()))
    }
    await p2.screenshot({ path: resolve(OUT, 'recipient', 'R-04-dup.png') })
  } finally { await p2.context().close() }
}, { page })

// ---------- B. 建立清單 ----------
await run.step('R-05', '建立清單頁表單檢查：類型、預設值、驚喜／日期驗證', async ({ note }) => {
  await page.goto('/lists/new'); await hydrated(page)
  await expect(page.getByRole('radio', { name: '心願清單' })).toBeChecked()
  const types = await page.getByRole('radio', { name: /清單$|登記|救助/ }).count()
  note(`清單類型 radio 僅 ${await page.locator('.c-seg input[type=radio]').count()} 個（心願清單）；規格列 personal/registry/relief 三型，UI 只開 personal`)
  await expect(page.getByRole('radio', { name: /^所有好友/ })).toBeChecked()
  note('預設可見度＝所有好友（friends）；預設購買期限：' + await page.getByLabel('購買期限').inputValue() + ' 小時')
  await page.getByLabel(/清單名稱/).fill('驗證用')
  await page.getByLabel('驚喜模式').check()
  await page.getByRole('button', { name: '建立並新增品項' }).click()
  await expect(page.getByRole('alert')).toHaveText('開啟驚喜模式需設定活動日')
  await page.getByLabel('活動日（選填）').fill(dateIn(0))
  await page.getByRole('button', { name: '建立並新增品項' }).click()
  await expect(page.getByRole('alert')).toHaveText('活動日需為今日之後')
  await page.getByLabel('驚喜模式').uncheck()
  await page.getByLabel('活動日（選填）').fill('')
  await page.getByLabel(/自動結束時間/).fill(localIn(-1))
  const under = await page.getByLabel(/自動結束時間/).evaluate(e => e.validity.rangeUnderflow)
  note(`自動結束時間填過去：瀏覽器原生 min 驗證 rangeUnderflow = ${under}（JS 的「結束時間需在現在之後」訊息在支援 min 的瀏覽器上不會出現，屬後備）`)
  expect(under).toBe(true)
  note('兩種前端驗證訊息（驚喜需活動日、活動日需為今日之後）皆正確')
  await page.getByLabel(/清單名稱/).fill('')
  await expect(page.getByRole('button', { name: '建立並新增品項' })).toBeDisabled()
  note('標題空白時送出鈕停用')
}, { page })

S.titleA = `滿月寶寶清單 ${tag}`
await run.step('R-06', '建立清單 A（公開、寶寶封面、活動日、顯示暱稱、說明）', async ({ note }) => {
  S.listA = await uiCreateList(page, { title: S.titleA, desc: '寶寶滿月，謝謝大家的心意', cover: '寶寶', event: dateIn(30), showNames: true, vis: '公開', ttl: '14 天' })
  note('清單 id=' + S.listA)
  await expect(page.getByText('還沒有品項，先新增第一個吧')).toBeVisible()
  await expect(page.getByRole('button', { name: '發佈並分享' })).toBeDisabled()
  note('草稿狀態、無品項時「發佈並分享」停用')
  S.api = await ownerLogin(owner.email)
  const j = await wl(S.api, S.listA)
  S.slugA = j.wishlist.slug
  note(`平台外：API 登入讀回 → visibility=${j.wishlist.visibility}, cover_preset=${j.wishlist.cover_preset}, show_claimer_names=${j.wishlist.show_claimer_names}, claim_ttl_hours=${j.wishlist.claim_ttl_hours}, status=${j.wishlist.status}`)
  expect(j.wishlist.visibility).toBe('public'); expect(j.wishlist.cover_preset).toBe('baby'); expect(j.wishlist.claim_ttl_hours).toBe(336)
  const draft = await (await apiCtx()).get(`public/wishlists/${S.slugA}`)
  note('草稿清單的公開 API → ' + draft.status() + '（分享面板也警告「尚未發佈，朋友打開會看到不存在」）'); expect(draft.status()).toBe(404)
}, { page })

await run.step('R-07', '新增數量制品項（品牌／規格／連結／價格／優先度高）與最小品項', async ({ note }) => {
  need('listA')
  let d = await uiAddItem(page, { title: '奶瓶組', brand: 'Pigeon', spec: '240ml 寬口徑', url: 'https://example.com/bottle', qty: 3, price: 450, prio: '高' })
  await saveItem(page, d)
  await expect(itemCard(page, '奶瓶組')).toContainText('需要 3')
  await expect(itemCard(page, '奶瓶組')).toContainText('優先度高')
  d = await uiAddItem(page, { title: '包巾', qty: 1, prio: '低' })
  await saveItem(page, d)
  await expect(itemCard(page, '包巾')).toContainText('優先度低')
  d = await uiAddItem(page, { title: '尿布', qty: 5 })
  await saveItem(page, d)
  await expect(itemCard(page, '尿布')).toContainText('優先度中')
  note('三個品項建立；預設優先度＝中；清單頁依序顯示')
  await page.getByRole('button', { name: '＋ 新增品項' }).click()
  const d2 = page.getByRole('dialog', { name: '新增品項' })
  await d2.getByRole('button', { name: '儲存品項' }).click()
  note('空白名稱送出：HTML required 擋下（dialog 仍開）= ' + await d2.isVisible())
  await d2.getByRole('button', { name: '取消' }).click()
}, { page })

await run.step('R-08', '品項圖片上傳（選圖 → 壓縮 → presign PUT → confirm → 處理中輪詢）', async ({ note }) => {
  need('listA')
  const d = await uiAddItem(page, { title: '嬰兒毯', qty: 1, image: PNG_ITEM })
  const st = await d.getByText(/已上傳，圖片處理完成後才會公開顯示|上傳失敗|圖片限|圖片需/).first().innerText()
  note('上傳結果文字：' + st)
  if (/失敗/.test(st)) { await d.getByRole('button', { name: '取消' }).click(); throw new Blocked('S3 替身（moto）上傳失敗：' + st) }
  await saveItem(page, d)
  await expect(itemCard(page, '嬰兒毯')).toBeVisible()
  await expect.poll(async () => await itemCard(page, '嬰兒毯').locator('img.c-thumb').count(), { timeout: 30000, message: '等待圖片處理完成' }).toBeGreaterThan(0).catch(() => {})
  const badge = await itemCard(page, '嬰兒毯').innerText()
  note('品項卡內容：' + badge.replace(/\n+/g, ' | '))
  const img = await itemCard(page, '嬰兒毯').locator('img.c-thumb').count()
  if (!img) throw new Gap('上傳成功但 30 秒內品項仍無縮圖：' + badge.replace(/\n+/g, ' | '))
}, { page })

await run.step('R-09', '新增眾籌品項：未填收件資訊被擋 → 填寫收件資訊 → 再建立；目標點數驗證', async ({ note }) => {
  need('listA')
  let d = await uiAddItem(page, { title: '嬰兒推車', crowd: { target: 3000, deadline: localIn(20), ref: 3200 }, prio: '高' })
  await saveItem(page, d)
  await expect(d.getByRole('alert')).toContainText('收件資訊')
  note('未填收件資訊：' + await d.getByRole('alert').innerText())
  await d.getByRole('button', { name: '填寫收件資訊' }).click()
  const ship = page.locator('details', { hasText: '收件資訊（心願眾籌用）' })
  await ship.getByLabel(/收件人/).fill('王小美')
  await ship.getByLabel(/電話/).fill('0912345678')
  await ship.getByLabel(/地址/).fill('台北市大安區和平東路二段 1 號 3 樓')
  await ship.getByRole('button', { name: '儲存收件資訊' }).click()
  await expect(ship.getByText('已儲存')).toBeVisible()
  note('收件資訊只顯示遮罩：' + (await ship.innerText()).replace(/\n+/g, ' | ').slice(0, 160))
  d = await uiAddItem(page, { title: '嬰兒推車', crowd: { target: 3000, deadline: localIn(20), ref: 3200 }, prio: '高' })
  await saveItem(page, d)
  await expect(itemCard(page, '嬰兒推車')).toContainText('心願眾籌 0 / 3,000 點')
  // 目標點數 0 / 負數
  await page.getByRole('button', { name: '＋ 新增品項' }).click()
  const d3 = page.getByRole('dialog', { name: '新增品項' })
  await d3.getByLabel('名稱（必填）').fill('負目標')
  await d3.getByRole('radio', { name: '心願眾籌' }).check()
  await d3.getByLabel(/目標點數/).fill('0')
  await d3.getByRole('button', { name: '儲存品項' }).click()
  note('目標點數 0：dialog 仍開（HTML min 驗證）= ' + await d3.isVisible())
  await d3.getByRole('button', { name: '取消' }).click()
}, { page })

await run.step('R-10', '編輯品項：改名稱／數量／優先度，儲存後清單與 API 一致', async ({ note }) => {
  need('listA')
  await itemCard(page, '包巾').getByRole('button', { name: '編輯' }).click()
  const d = page.getByRole('dialog', { name: '編輯品項' })
  await d.getByLabel('名稱（必填）').fill('純棉包巾')
  await d.getByLabel(/需要數量/).fill('2')
  await d.getByRole('radio', { name: '高', exact: true }).check()
  await d.getByRole('button', { name: '儲存品項' }).click()
  await expect(itemCard(page, '純棉包巾')).toContainText('需要 2')
  await expect(itemCard(page, '純棉包巾')).toContainText('優先度高')
  note('編輯成功')
  // 眾籌品項切成數量制（尚無贊助）
  await itemCard(page, '嬰兒推車').getByRole('button', { name: '編輯' }).click()
  const d2 = page.getByRole('dialog', { name: '編輯品項' })
  note('眾籌品項編輯：取得方式 radio 是否可切換 = ' + await d2.getByRole('radio', { name: '直接送禮' }).isEnabled())
  await d2.getByRole('button', { name: '取消' }).click()
}, { page })

await run.step('R-11', '品項排序（↑↓）後重整仍保留；頭尾按鈕停用', async ({ note }) => {
  need('listA')
  const names = () => page.locator('article.c-card strong').allInnerTexts()
  const before = await names(); note('排序前：' + before.join(' > '))
  const card = itemCard(page, '嬰兒推車')
  for (let k = before.indexOf('嬰兒推車'); k > 0; k--) await itemCard(page, '嬰兒推車').getByRole('button', { name: '上移' }).click()
  await expect.poll(async () => (await names())[0]).toBe('嬰兒推車')
  await expect(itemCard(page, '嬰兒推車').getByRole('button', { name: '上移' })).toBeDisabled()
  await sleep(600); await page.reload(); await hydrated(page)
  await expect(page.locator('article.c-card').first()).toBeVisible()
  const after = await names(); note('重整後：' + after.join(' > '))
  expect(after[0]).toBe('嬰兒推車')
}, { page })

await run.step('R-12', '刪除無人認領的品項（確認對話框）', async ({ note }) => {
  need('listA')
  await itemCard(page, '嬰兒毯').getByRole('button', { name: '刪除' }).click()
  const dlg = page.getByRole('dialog', { name: '刪除品項' })
  await expect(dlg).toContainText('確定刪除「嬰兒毯」？')
  await dlg.getByRole('button', { name: '取消' }).click()
  await expect(itemCard(page, '嬰兒毯')).toBeVisible()
  await itemCard(page, '嬰兒毯').getByRole('button', { name: '刪除' }).click()
  await dlg.getByRole('button', { name: '刪除', exact: true }).click()
  await expect(itemCard(page, '嬰兒毯')).toHaveCount(0)
  note('取消不刪、確認後刪除成功')
}, { page })

await run.step('R-13', '清單設定：改名稱／說明／活動日／結束時間／封面／購買期限；重整後仍在', async ({ note }) => {
  need('listA')
  await page.getByText('清單設定', { exact: true }).click()
  const f = page.locator('details', { hasText: '儲存設定' })
  S.titleA2 = `${S.titleA}（更新）`
  await f.getByLabel('清單名稱').fill(S.titleA2)
  await f.getByLabel('說明', { exact: true }).fill('更新後的說明')
  await f.getByLabel('自動結束時間（選填，留空＝不設期限）').fill(localIn(25))
  await f.getByRole('radio', { name: /生日/ }).check({ force: true })
  await f.getByLabel('購買期限').selectOption({ label: '30 天' })
  const [resp] = await Promise.all([page.waitForResponse(r => r.request().method() === 'PATCH' && /wishlists\//.test(r.url())), f.getByRole('button', { name: '儲存設定' }).click()])
  note('PATCH 狀態 ' + resp.status())
  await expect(f.getByText('已儲存')).toBeVisible()
  await page.reload(); await hydrated(page)
  await expect(page.getByRole('heading', { level: 1, name: S.titleA2 })).toBeVisible()
  await page.getByText('清單設定', { exact: true }).click()
  await expect(page.getByLabel('自動結束時間（選填，留空＝不設期限）')).toHaveValue(localIn(25))
  note('重整後結束時間仍為 ' + localIn(25))
  const j = await wl(S.api, S.listA)
  note(`API：cover_preset=${j.wishlist.cover_preset}, claim_ttl_hours=${j.wishlist.claim_ttl_hours}`)
  expect(j.wishlist.cover_preset).toBe('birthday'); expect(j.wishlist.claim_ttl_hours).toBe(720)
}, { page })

await run.step('R-13b', '兩個分頁同時編輯清單設定：後儲存者收到「已在其他地方被修改」並可重新載入（樂觀鎖）', async ({ note }) => {
  need('listA')
  const p2 = await newPage({ user: { api: S.api } })
  try {
    await p2.goto(`/lists/${S.listA}/edit`); await hydrated(p2)
    await p2.getByText('清單設定', { exact: true }).click()
    const cur = await wl(S.api, S.listA)
    const r = await S.api.patch(`wishlists/${S.listA}`, { data: { description: '更新後的說明 v2', expected_updated_at: cur.wishlist.updated_at } })
    note('平台外：另一個「分頁」（API）先改說明 → ' + r.status())
    const f2 = p2.locator('details', { hasText: '儲存設定' })
    await f2.getByLabel('說明', { exact: true }).fill('第二個分頁的說明')
    await f2.getByRole('button', { name: '儲存設定' }).click()
    await expect(p2.getByRole('alert')).toContainText('已在其他地方被修改')
    note('第二個分頁儲存 → ' + await p2.getByRole('alert').innerText())
    await p2.getByRole('button', { name: '重新載入' }).click()
    await p2.getByText('清單設定', { exact: true }).click().catch(() => {})
    await expect(p2.getByLabel('說明', { exact: true })).toHaveValue('更新後的說明 v2')
    note('按「重新載入」後取得最新內容（更新後的說明 v2），未覆蓋他人修改')
  } finally { await p2.context().close() }
  await page.reload(); await hydrated(page) // 主頁面的版本已過期，重載後再繼續
}, { page })

await run.step('R-14', '封面上傳自己的照片（上傳後取代預設圖）', async ({ note }) => {
  need('listA')
  const f = page.locator('details', { hasText: '儲存設定' })
  if (!(await f.getAttribute('open') !== null)) await page.getByText('清單設定', { exact: true }).click()
  await f.locator('input[type=file]').setInputFiles(PNG_COVER)
  await expect(f.getByText(/已上傳|上傳失敗/).first()).toBeVisible({ timeout: 15000 })
  const msg = await f.getByText(/已上傳|上傳失敗/).first().innerText(); note('上傳結果：' + msg)
  if (/失敗/.test(msg)) throw new Blocked('S3 替身上傳失敗：' + msg)
  await f.getByRole('button', { name: '儲存設定' }).click()
  await expect(f.getByText('已儲存')).toBeVisible()
  const j = await wl(S.api, S.listA)
  note(`API：cover_image_url=${j.wishlist.cover_image_url ? '有' : '無'}, cover_preset=${j.wishlist.cover_preset}`)
  if (!j.wishlist.cover_image_url) throw new Gap('儲存後沒有 cover_image_url（可能仍在處理）')
}, { page })

await run.step('R-15', '發佈並分享：分享面板（連結、複製、QR、LINE／Email／FB 連結、openExternalBrowser）', async ({ note }) => {
  need('listA')
  const { dlg, url, slug } = await publish(page)
  S.shareUrl = url; S.slugA = slug
  note('分享連結：' + url)
  expect(url).toMatch(/\/s\/[A-Za-z0-9]{10}$/)
  await dlg.getByRole('button', { name: '複製連結' }).click()
  await expect(dlg.getByRole('button', { name: '已複製 ✓' })).toBeVisible()
  const clip = await page.evaluate(() => navigator.clipboard.readText()).catch(e => 'ERR ' + e.message)
  note('剪貼簿內容 = ' + clip); expect(clip).toBe(url)
  await expect(dlg.locator('svg, canvas, img').first()).toBeVisible()
  note('QR code 元件可見')
  const hrefs = await dlg.getByRole('link').evaluateAll(a => a.map(x => `${x.textContent.trim()}→${x.getAttribute('href').slice(0, 60)}`))
  note('分享連結：' + hrefs.join(' ; '))
  await dlg.getByLabel(/openExternalBrowser/).check()
  const url2 = await dlg.locator('#c-share-input').inputValue(); note('勾選外部瀏覽器後：' + url2)
  expect(url2).toContain('openExternalBrowser=1')
  await dlg.getByLabel(/openExternalBrowser/).uncheck()
  await expect(dlg.getByText('附上好友邀請')).toHaveCount(0)
  note('公開清單不顯示「附上好友邀請」（只有 friends／selected 才有）')
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-15-share.png'), fullPage: true })
  await closeSheet(dlg)
  await expect(page.getByRole('button', { name: '分享', exact: true })).toBeVisible()
  const j = await wl(S.api, S.listA); expect(j.wishlist.status).toBe('active'); note('狀態 active')
}, { page })

// ---------- C. 分享頁 ----------
await run.step('R-16', '分享頁 /s/[slug]（匿名桌面）：標題、說明、品項資訊、完成度、OG meta', async ({ note }) => {
  need('slugA')
  const anon = await newPage()
  try {
    const res = await anon.goto(`/s/${S.slugA}`); await hydrated(anon)
    note('HTTP ' + res.status())
    await expect(anon.getByRole('heading', { level: 1, name: S.titleA2 })).toBeVisible()
    await expect(anon.getByText('更新後的說明')).toBeVisible()
    const card = anon.locator('li.g-card', { hasText: '奶瓶組' })
    const txt = (await card.innerText()).replace(/\n+/g, ' | ')
    note('奶瓶組卡片：' + txt)
    for (const k of ['Pigeon', '240ml', '450']) if (!txt.includes(k)) { note(`卡片未顯示「${k}」`); if (k === '450') S.priceHidden = true }
    note('嬰兒推車卡片：' + (await anon.locator('li.g-card', { hasText: '嬰兒推車' }).innerText()).replace(/\n+/g, ' | '))
    const og = await anon.evaluate(() => Object.fromEntries([...document.querySelectorAll('meta[property^="og:"]')].map(m => [m.getAttribute('property'), m.content])))
    note('OG：' + JSON.stringify(og))
    expect(og['og:title']).toContain(S.titleA2)
    note('匿名按「我來送」→ ' + (await card.getByRole('link', { name: /登入|我來送/ }).first().getAttribute('href').catch(() => '(button)')))
    await anon.screenshot({ path: resolve(OUT, 'recipient', 'R-16-desktop.png'), fullPage: true })
    S.coverShown = await anon.locator('img.g-cover').getAttribute('src'); note('封面 src=' + S.coverShown)
  } finally { await anon.context().close() }
  if (S.priceHidden) throw new Gap('建立者填的「參考價格 NT$450」API 有回傳（public.rs:344 unit_price_amount），但分享頁品項卡沒有顯示價格（components/guest/ItemCard.vue 無 price）；好友動態 /friends 卻有顯示價格')
}, { page })

await run.step('R-16b', '分享頁 SSR 與瀏覽器的結束時間顯示一致（時區）：不得有 hydration 不一致', async ({ note }) => {
  need('slugA')
  const { request } = await import('@playwright/test')
  const api = await request.newContext()
  const html = await (await api.get(`${WEB}/s/${S.slugA}`)).text(); await api.dispose()
  const ssr = html.match(/(20\d\d年\d+月\d+日[^<]{0,12}) 結束・/)?.[1]
  const anon = await newPage()
  const warns = []; anon.on('console', m => { if (/hydrat/i.test(m.text())) warns.push(m.text().slice(0, 120)) })
  try {
    await anon.goto(`/s/${S.slugA}`); await hydrated(anon); await anon.waitForTimeout(500)
    const cli = (await anon.getByText(/結束・/).first().innerText()).match(/(20\d\d年\d+月\d+日[^結]{0,12}) 結束/)?.[1]
    const truth = new Date(sql(`SELECT closes_at FROM wishlists WHERE slug = '${S.slugA}'`)).toLocaleString('zh-TW', { timeZone: 'Asia/Taipei', dateStyle: 'medium', timeStyle: 'short' })
    note(`伺服器 SSR 文字：「${ssr}」；瀏覽器（Asia/Taipei）hydrate 後：「${cli}」；DB closes_at 的台北時間：「${truth}」`)
    note('hydration 警告：' + JSON.stringify(warns))
    if (ssr !== cli || warns.length) throw new Error(`SSR 與瀏覽器顯示的結束時間不同（SSR「${ssr}」vs 瀏覽器「${cli}」），且有 hydration 警告 ${warns.length} 則：[slug].vue 的 toLocaleString 未指定 timeZone`)
  } finally { await anon.context().close() }
}, { page })

await run.step('R-17', '分享頁手機 390 寬：無橫向捲動、品項可讀', async ({ note }) => {
  need('slugA')
  const m = await newPage({ mobile: true })
  try {
    await m.goto(`/s/${S.slugA}`); await hydrated(m)
    await expect(m.getByRole('heading', { level: 1, name: S.titleA2 })).toBeVisible()
    const ok = await noHScroll(m); note('無橫向捲動 = ' + ok); expect(ok).toBe(true)
    await m.screenshot({ path: resolve(OUT, 'recipient', 'R-17-mobile.png'), fullPage: true })
    // 創建者頁面手機版
    await uiLogin(m, owner.email); await hydrated(m)
    for (const p of ['/dashboard', `/lists/${S.listA}/edit`, `/lists/${S.listA}/progress`, '/settings', '/friends']) {
      await m.goto(p); await hydrated(m)
      const good = await noHScroll(m); note(`手機 ${p.replace(S.listA, ':id')} 無橫向捲動 = ${good}`)
      if (!good) throw new Gap(`手機寬度 ${p} 出現橫向捲動`)
    }
    await m.goto('/dashboard'); await hydrated(m)
    await m.screenshot({ path: resolve(OUT, 'recipient', 'R-17-dashboard-mobile.png'), fullPage: true })
  } finally { await m.context().close() }
}, { page })

await run.step('R-18', '儀表板：清單卡（狀態、完成度）、進度連結', async ({ note }) => {
  await page.goto('/dashboard'); await hydrated(page)
  const card = page.locator('.c-card', { hasText: S.titleA2 })
  await expect(card).toBeVisible()
  note('卡片：' + (await card.innerText()).replace(/\n+/g, ' | '))
  await expect(card).toContainText('進行中')
  await expect(card).toContainText('完成度 0%')
}, { page })

// ---------- D. 進度與認捐 ----------
await run.step('R-19', '進度頁（尚無人認捐）：空狀態與「分享清單」引導', async ({ note }) => {
  need('listA')
  await page.goto(`/lists/${S.listA}/progress`); await hydrated(page)
  await expect(page.getByText('還沒有人認領或贊助，去分享連結吧')).toBeVisible()
  await expect(page.getByRole('link', { name: '分享清單' })).toBeVisible()
  await expect(page.getByText('即時更新中')).toBeVisible({ timeout: 15000 })
  note('SSE 即時更新連線建立（顯示「即時更新中」）')
}, { page })

await run.step('R-20', '製造資料：捐助者 A/B 認捐、營運發點、捐助者贊助眾籌（平台外）', async ({ note }) => {
  need('listA')
  S.donorA = await registerApi({ name: `捐助者小明${tag.slice(0, 3)}`, tag: 'don' })
  S.donorB = await registerApi({ name: `捐助者小華${tag.slice(0, 3)}`, tag: 'don' })
  S.staff = await registerApi({ name: `營運${tag.slice(0, 3)}`, tag: 'stf', staff: true })
  note('平台外：以 API 註冊兩位捐助者與一位營運者（本流程專用，已用 SQL 設 is_staff）')
  const j = await wl(S.api, S.listA)
  S.items = Object.fromEntries(j.items.map(i => [i.title, i.id]))
  const bottle = S.items['奶瓶組'], diaper = S.items['尿布'], cart = S.items['嬰兒推車']
  S.claimA = (await claimApi(S.donorA, bottle, { qty: 2, note: '我買 Pigeon 的' })).claim ?? null
  await page.waitForTimeout(100)
  S.claimB = (await claimApi(S.donorB, bottle, { qty: 1, is_anonymous: true })).claim ?? null
  S.claimC = (await claimApi(S.donorA, diaper, { qty: 5 })).claim ?? null
  note(`認捐：A×2＋B(匿名)×1 奶瓶組、A×5 尿布；claim ids=${[S.claimA?.id, S.claimB?.id, S.claimC?.id].map(x => x?.slice(0, 8))}`)
  const uidA = await meId(S.donorA.api)
  const g = await S.staff.api.post('admin/wallets/grants', { data: { user_id: uidA, points: 3000, reason: 'QA 受捐者流程' }, headers: { 'Idempotency-Key': randomUUID() } })
  if (!g.ok()) throw new Error('grant ' + g.status() + await g.text())
  const uidB = await meId(S.donorB.api)
  const g2 = await S.staff.api.post('admin/wallets/grants', { data: { user_id: uidB, points: 1000, reason: 'QA 受捐者流程' }, headers: { 'Idempotency-Key': randomUUID() } })
  if (!g2.ok()) throw new Error('grant2 ' + g2.status())
  note('營運（API）發點：A 3000、B 1000')
}, { page })

await run.step('R-21', '進度頁即時更新：認捐後不重整就出現；姓名、匿名、備註、完成度', async ({ note }) => {
  need('items')
  await expect(page.getByText('3 / 3').first()).toBeVisible({ timeout: 20000 }).catch(() => {})
  await page.waitForTimeout(1500)
  const art = page.locator('article.c-card', { hasText: '奶瓶組' })
  const txt = (await art.innerText()).replace(/\n+/g, ' | ')
  note('奶瓶組：' + txt)
  await expect(art).toContainText(S.donorA.name)
  await expect(art).toContainText('×2')
  await expect(art).toContainText('匿名朋友')
  await expect(art).toContainText('我買 Pigeon 的')
  note('整體完成度：' + await page.locator('.pg-hero .x-num').innerText())
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-21-progress.png'), fullPage: true })
  const live = await page.getByText('即時更新中').count()
  note('「即時更新中」顯示 = ' + live)
}, { page })

await run.step('R-22', '公開分享頁看到的認捐資訊：顯示暱稱開啟（A）、匿名者不洩漏、品項滿額', async ({ note }) => {
  need('slugA')
  const anon = await newPage()
  try {
    await anon.goto(`/s/${S.slugA}`); await hydrated(anon)
    const bottle = anon.locator('li.g-card', { hasText: '奶瓶組' })
    note('奶瓶組：' + (await bottle.innerText()).replace(/\n+/g, ' | '))
    await expect(bottle).toContainText('3 / 3')
    const body = await anon.locator('main').innerText()
    note('頁面是否出現捐助者名字 A = ' + body.includes(S.donorA.name) + '；B(匿名) = ' + body.includes(S.donorB.name))
    expect(body.includes(S.donorB.name)).toBe(false)
    note('尿布：' + (await anon.locator('li.g-card', { hasText: '尿布' }).innerText()).replace(/\n+/g, ' | '))
    const vis = sql(`SELECT string_agg(visibility::text || '/' || is_anonymous, ', ') FROM claims WHERE item_id = '${S.items['奶瓶組']}'`)
    note('平台外：DB claims.visibility/is_anonymous = ' + vis + '（捐助者預設送禮紀錄可見度為「僅好友」，所以公開頁把 A 也顯示成「匿名」）')
    // 捐助者 A 把這筆認領改成 public → 公開頁才顯示名字
    const pr = await S.donorA.api.patch(`claims/${S.claimA.id}`, { data: { visibility: 'public' } })
    note('捐助者 A 將認領可見度改 public → PATCH ' + pr.status())
    await anon.reload(); await hydrated(anon)
    const t2 = (await anon.locator('li.g-card', { hasText: '奶瓶組' }).innerText()).replace(/\n+/g, ' | ')
    note('改 public 後奶瓶組卡片：' + t2)
    expect(t2.includes(S.donorA.name)).toBe(true)
    S.nameNeedsDonorPublic = true
  } finally { await anon.context().close() }
  throw new Gap('建立者開了「顯示送禮者暱稱給其他訪客」，但捐助者預設送禮紀錄可見度是「僅好友」，公開頁仍顯示「匿名」；建立者沒有任何提示（public.rs:276 依 claims.visibility 判斷），開關在預設情況下形同無效')
}, { page })

await run.step('R-23', '標記「已送達」（收到禮物）：確認對話框、狀態變更、不可復原', async ({ note }) => {
  need('claimA')
  await page.goto(`/lists/${S.listA}/progress`); await hydrated(page)
  const row = page.locator('article.c-card', { hasText: '尿布' }).locator('li', { hasText: S.donorA.name })
  let dlgMsg = ''
  page.once('dialog', d => { dlgMsg = d.message(); d.dismiss() })
  await row.getByRole('button', { name: '標為已送達' }).click()
  note('瀏覽器 confirm 文字：' + dlgMsg)
  await expect(row).toContainText('已認領')
  note('取消 confirm → 狀態不變')
  page.once('dialog', d => d.accept())
  await row.getByRole('button', { name: '標為已送達' }).click()
  await expect(row).toContainText('已送達', { timeout: 10000 })
  await expect(row.getByRole('button', { name: '標為已送達' })).toHaveCount(0)
  await expect(row.getByRole('button', { name: '取消認領' })).toHaveCount(0)
  note('已送達後兩個按鈕消失（不可復原）')
  const g = await S.donorA.api.get('guest/me?limit=50').then(r => r.json()).catch(() => null)
  note('捐助者端狀態（API guest/me）：' + JSON.stringify((g?.claims ?? []).filter(c => c.claim.id === S.claimC?.id).map(c => c.claim.status)))
}, { page })

await run.step('R-24', '取消認領（受捐者代為取消）釋出數量，捐助者收到通知', async ({ note }) => {
  need('claimB')
  const art = page.locator('article.c-card', { hasText: '奶瓶組' })
  const row = art.locator('li', { hasText: '匿名朋友' })
  let msg = ''
  page.once('dialog', d => { msg = d.message(); d.accept() })
  await row.getByRole('button', { name: '取消認領' }).click()
  await expect(row).toContainText('已取消', { timeout: 10000 })
  note('confirm：' + msg)
  await expect(art.locator('span', { hasText: /^2 \/ 3$/ })).toBeVisible({ timeout: 10000 })
  note('奶瓶組數量回到 2 / 3')
  const mails = await listMail(S.donorB.email)
  note(`捐助者 B 信箱主旨：${mails.map(m => m.subject).join(' | ')}`)
  const n = sql(`SELECT count(*) FROM notifications WHERE user_id = (SELECT id FROM users WHERE email = '${q(S.donorB.email)}') AND kind NOT IN ('auth.otp', 'claim.purchase_reminder')`)
  note('平台外：捐助者 B 的 notifications 佇列筆數（排除 OTP）= ' + n)
  const dp = await newPage({ user: S.donorB })
  try { await dp.goto('/me/claims'); await hydrated(dp); const t = (await dp.locator('main').innerText()).replace(/\n+/g, ' | '); note('捐助者 B 「我的送禮」頁：' + t.slice(0, 300)) } finally { await dp.context().close() }
  note('受捐者取消時 UI 沒有要求填原因（規格 02：owner 取消時 audit 記錄原因）')
  if (n === '0') throw new Gap('受捐者取消他人認領後，捐助者沒有收到任何通知（規格 02「P1 通知捐助者」；notify.rs 沒有 claim.cancelled_by_owner 類型），且取消介面沒有原因欄位')
}, { page })

await run.step('R-25', '「感謝」功能：受捐者端是否有感謝訊息／謝卡入口', async ({ note }) => {
  await page.goto(`/lists/${S.listA}/progress`); await hydrated(page)
  const body = await page.locator('main').innerText()
  const hits = (body.match(/感謝|謝卡|謝謝|thank/gi) ?? [])
  note(`進度頁含「感謝／謝卡」字樣次數 = ${hits.length}`)
  const codeHits = (await import('node:child_process')).execFileSync('bash', ['-c', `grep -rln "thank\\|感謝\\|謝卡" /home/user/wishly/apps/web/app /home/user/wishly/apps/api/src | head`], { encoding: 'utf8' }).trim()
  note('程式碼中含 thank／感謝／謝卡 的檔案：' + (codeHits || '(無)'))
  if (!hits.length) throw new Gap('規格 03 §受捐者旅程「活動日後 驚喜解鎖，查看誰送了什麼，寫謝卡」，但產品沒有任何謝卡／感謝入口；受捐者只能標記已送達')
}, { page })

// ---------- E. 眾籌 ----------
await run.step('R-26', '眾籌品項收到點數：進度條、贊助者名單（含匿名、留言）、尚未達標狀態', async ({ note }) => {
  need('items')
  const cart = S.items['嬰兒推車']
  const r1 = await S.donorA.api.post(`items/${cart}/contributions`, { data: { points: 1200, message: '推車加油' }, headers: { 'Idempotency-Key': randomUUID() } })
  if (!r1.ok()) throw new Error('pledge ' + r1.status() + await r1.text())
  const r2 = await S.donorB.api.post(`items/${cart}/contributions`, { data: { points: 500, is_anonymous: true }, headers: { 'Idempotency-Key': randomUUID() } })
  if (!r2.ok()) throw new Error('pledge2 ' + r2.status() + await r2.text())
  note('平台外：捐助者 A 贊助 1200（留言）、B 匿名贊助 500')
  await page.goto(`/lists/${S.listA}/progress`); await hydrated(page)
  const art = page.locator('article.c-card', { hasText: '嬰兒推車' })
  await expect(art).toContainText('已募 1,700 / 3,000 點', { timeout: 15000 })
  await expect(art).toContainText('推車加油')
  await expect(art).toContainText('匿名朋友')
  const txt = (await art.innerText()).replace(/\n+/g, ' | '); note('眾籌卡：' + txt)
  expect(txt.includes(S.donorB.name)).toBe(false)
  note('整體：' + (await page.locator('.pg-hero .c-mute').first().innerText()))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-26-crowd.png'), fullPage: true })
}, { page })

await run.step('R-27', '眾籌達標 → 採購單（營運推進 placed/shipped/delivered）→ 受捐者進度頁看到物流', async ({ note }) => {
  need('items')
  const cart = S.items['嬰兒推車']
  const r = await S.donorA.api.post(`items/${cart}/contributions`, { data: { points: 1300 }, headers: { 'Idempotency-Key': randomUUID() } })
  if (!r.ok()) throw new Error('pledge3 ' + r.status() + await r.text())
  note('平台外：A 再贊助 1300 → 合計 3000 達標')
  await page.reload(); await hydrated(page)
  const art = page.locator('article.c-card', { hasText: '嬰兒推車' })
  await expect(art).toContainText('已達標', { timeout: 15000 })
  note('達標後狀態：' + (await art.innerText()).replace(/\n+/g, ' | ').slice(0, 300))
  const list = await (await S.staff.api.get('admin/orders?status=pending&limit=50')).json().catch(() => ({}))
  const ord = (list.data ?? list.orders ?? []).find(o => (o.item?.id ?? o.item_id) === cart)
  if (!ord) throw new Gap('營運 API 找不到對應採購單：' + JSON.stringify(list).slice(0, 200))
  for (const [status, extra] of [['placed', { merchant_order_id: 'QA-' + tag, amount: 3000 }], ['shipped', { tracking_no: 'TW' + tag }], ['delivered', {}]]) {
    const p = await S.staff.api.patch(`admin/orders/${ord.id}`, { data: { status, ...extra } })
    if (!p.ok()) throw new Error(`order ${status} ${p.status()} ${await p.text()}`)
    note(`平台外：營運（API）將採購單推進為 ${status}`)
  }
  await page.reload(); await hydrated(page)
  await expect(art).toContainText('TW' + tag, { timeout: 15000 })
  note('進度頁採購進度：' + (await art.innerText()).replace(/\n+/g, ' | ').slice(-220))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-27-order.png'), fullPage: true })
  const dsh = await wl(S.api, S.listA)
  S.cartDone = true
}, { page })

await run.step('R-28', '眾籌品項達標後：編輯受限（不可刪除？不可改目標）', async ({ note }) => {
  need('cartDone')
  await page.goto(`/lists/${S.listA}/edit`); await hydrated(page)
  await itemCard(page, '嬰兒推車').getByRole('button', { name: '編輯' }).click()
  const d = page.getByRole('dialog', { name: '編輯品項' })
  note('目標點數欄停用 = ' + await d.getByLabel(/目標點數/).isDisabled() + '；取得方式 radio 停用 = ' + await d.getByRole('radio', { name: '直接送禮' }).isDisabled())
  note('提示：' + (await d.getByText(/已達標或截止|已有人認領或贊助/).allInnerTexts()).join(' / '))
  await d.getByRole('button', { name: '取消' }).click()
  await itemCard(page, '嬰兒推車').getByRole('button', { name: '刪除' }).click()
  const dlg = page.getByRole('dialog', { name: '刪除品項' })
  await dlg.getByRole('button', { name: '刪除', exact: true }).click()
  await page.waitForTimeout(1200)
  const alert = await dlg.getByRole('alert').allInnerTexts()
  note('嘗試刪除已達標眾籌品項 → 對話框訊息：' + JSON.stringify(alert) + '；品項是否還在 = ' + (await itemCard(page, '嬰兒推車').count()))
  const first = alert[0] ?? ''
  const cartId = S.items['嬰兒推車']
  const direct = await S.api.delete(`items/${cartId}`)
  note(`API 直接 DELETE → ${direct.status()} ${(await direct.text()).slice(0, 140)}`)
  await dlg.getByRole('button', { name: '連同認領一併刪除' }).click()
  await page.waitForTimeout(1200)
  const second = await dlg.getByRole('alert').allInnerTexts()
  note('點「連同認領一併刪除」後訊息：' + JSON.stringify(second) + '；品項仍在 = ' + (await itemCard(page, '嬰兒推車').count()))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-28-delete-funded.png') })
  if (await dlg.isVisible()) await dlg.getByRole('button', { name: '取消' }).click()
  expect(await itemCard(page, '嬰兒推車').count()).toBe(1)
  if (/已有人認領/.test(first) && /採購|達標/.test(second.join())) throw new Gap('已達標且進入採購的眾籌品項按「刪除」，第一次訊息說「已有人認領。若仍要刪除，認領將一併取消」並提供「連同認領一併刪除」，但實際永遠刪不掉（API 409「品項已達標並進入採購，無法刪除」）；前端 remove() 對 ITEM_HAS_CLAIMS 一律顯示通用文案（lists/[id]/edit.vue:189）')
}, { page })

// ---------- F. 修改品項後的變更通知 ----------
await run.step('R-29', '修改已有人認領的品項 → 捐助者看到「建立者修改了品項」提醒並收到合併通知信', async ({ note }) => {
  need('items')
  await page.goto(`/lists/${S.listA}/edit`); await hydrated(page)
  await itemCard(page, '奶瓶組').getByRole('button', { name: '編輯' }).click()
  const d = page.getByRole('dialog', { name: '編輯品項' })
  const min = await d.getByLabel(/需要數量/).getAttribute('min'); note('數量欄 min=' + min + '（已認領 2，不可低於已認領數）')
  await d.getByLabel('規格（選填）').fill('240ml 寬口徑（改為 330ml）')
  await d.getByRole('button', { name: '儲存品項' }).click()
  await expect(itemCard(page, '奶瓶組')).toBeVisible()
  const dp = await newPage({ user: S.donorA })
  try {
    await dp.goto('/me/claims'); await hydrated(dp)
    const card = dp.locator('.g-item', { hasText: '奶瓶組' })
    await expect(card.locator('.js-item-changed')).toContainText('修改了這個品項')
    note('捐助者「我的送禮」顯示變更提醒')
    await dp.screenshot({ path: resolve(OUT, 'recipient', 'R-29-donor.png'), fullPage: true })
  } finally { await dp.context().close() }
  sql(`UPDATE notifications SET scheduled_at = now() WHERE kind = 'claim.item_changed' AND status = 'pending' AND payload->>'item_id' = '${S.items['奶瓶組']}'`)
  note('平台外：把延後 10 分鐘合併寄送的通知排程提前（UPDATE notifications.scheduled_at）')
  const mail = await waitForMail(S.donorA.email, { subject: /內容有更新/, timeout: 30000 })
  note('捐助者收到信：' + mail.subject)
}, { page })

await run.step('R-30', '刪除已有人認領的品項：第一次提示、第二次「連同認領一併刪除」，捐助者收到通知', async ({ note }) => {
  need('items')
  // 先加一個專門被刪的品項並由捐助者 B 認領
  const d = await uiAddItem(page, { title: '待刪品項', qty: 2 }); await saveItem(page, d)
  await expect(itemCard(page, '待刪品項')).toBeVisible()
  const id = (await wl(S.api, S.listA)).items.find(i => i.title === '待刪品項').id
  await claimApi(S.donorB, id, { qty: 1 })
  await itemCard(page, '待刪品項').getByRole('button', { name: '刪除' }).click()
  const dlg = page.getByRole('dialog', { name: '刪除品項' })
  await dlg.getByRole('button', { name: '刪除', exact: true }).click()
  await expect(dlg.getByRole('alert')).toContainText('已有人認領')
  note('第一次：' + await dlg.getByRole('alert').innerText())
  await dlg.getByRole('button', { name: '連同認領一併刪除' }).click()
  await expect(itemCard(page, '待刪品項')).toHaveCount(0)
  const mail = await waitForMail(S.donorB.email, { subject: /移除|取消|刪除|更新/, timeout: 30000 }).catch(() => null)
  note('捐助者 B 收到信：' + (mail?.subject ?? '(30 秒內沒有相關主旨的信；全部主旨：' + (await listMail(S.donorB.email)).map(m => m.subject).join(' | ') + ')'))
}, { page })

await run.step('R-30b', '靈感頁（選物目錄）→「加到我的願望清單」→ 品項新增面板預填 → 儲存', async ({ note }) => {
  need('listA')
  await page.goto('/inspire'); await hydrated(page)
  const add = page.getByRole('button', { name: '加到我的願望清單' }).first()
  if (!(await add.count())) throw new Blocked('/inspire 目錄是空的（需營運在 /admin/catalog 上架商品）')
  const title = await page.locator('.x-title').first().innerText()
  note('目錄第一個商品：' + title)
  await add.click()
  const pick = page.getByRole('dialog').filter({ hasText: '加到哪一份清單' })
  await expect(pick).toBeVisible()
  note('可選清單（草稿與進行中）：' + (await pick.getByRole('button').allInnerTexts()).join(' | ').slice(0, 200))
  await pick.getByRole('button', { name: S.titleA2 }).click()
  await page.waitForURL(/\/lists\/[0-9a-f-]+\/edit/); await hydrated(page)
  const sheet = page.getByRole('dialog', { name: '新增品項' })
  await expect(sheet).toBeVisible()
  expect(await sheet.getByLabel('名稱（必填）').inputValue()).toBe(title)
  note(`新增面板預填：名稱=${await sheet.getByLabel('名稱（必填）').inputValue()}、連結=${await sheet.getByLabel('商品網址（選填）').inputValue()}、價格=${await sheet.getByLabel(/參考價格/).inputValue()}`)
  await saveItem(page, sheet)
  await expect(itemCard(page, title)).toBeVisible()
  expect(page.url()).not.toContain('add=1'); note('儲存後網址已清掉 ?add=1：' + page.url())
}, { page })

// ---------- G. 驚喜模式 ----------
S.titleB = `驚喜派對 ${tag}`
await run.step('R-31', '驚喜模式清單 B（活動日 +7 天）：建立、發佈、自己看分享頁被遮蔽、進度頁只見數量', async ({ note }) => {
  S.listB = await uiCreateList(page, { title: S.titleB, event: dateIn(7), surprise: true, vis: '公開' })
  let d = await uiAddItem(page, { title: '生日蛋糕', qty: 1 }); await saveItem(page, d); await expect(itemCard(page, '生日蛋糕')).toBeVisible()
  d = await uiAddItem(page, { title: '派對帽', qty: 4 }); await saveItem(page, d); await expect(itemCard(page, '派對帽')).toBeVisible()
  const { dlg, slug } = await publish(page); S.slugB = slug; await closeSheet(dlg)
  await page.getByText('清單設定', { exact: true }).click()
  await expect(page.getByText('驚喜模式：開啟中（鎖定至活動日，無法關閉）')).toBeVisible()
  const items = (await wl(S.api, S.listB)).items
  S.itemsB = Object.fromEntries(items.map(i => [i.title, i.id]))
  await claimApi(S.donorA, S.itemsB['派對帽'], { qty: 2 })
  await claimApi(S.donorB, S.itemsB['生日蛋糕'], { qty: 1 })
  note('平台外：捐助者 A 認 派對帽×2、B 認 生日蛋糕×1')
  await page.goto(`/s/${S.slugB}`); await hydrated(page)
  await expect(page.getByText(/驚喜/).first()).toBeVisible()
  const sBody = await page.locator('main').innerText()
  note('自己開驚喜清單分享頁：含「驚喜」提示；是否出現捐助者名字 = ' + (sBody.includes(S.donorA.name) || sBody.includes(S.donorB.name)))
  const cardBtn = await page.locator('li.g-card', { hasText: '派對帽' }).getByRole('button').allInnerTexts()
  note('品項卡按鈕：' + JSON.stringify(cardBtn))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-31-share-owner.png'), fullPage: true })
  await page.goto(`/lists/${S.listB}/progress`); await hydrated(page)
  await expect(page.getByText('驚喜模式').first()).toBeVisible()
  const pBody = await page.locator('main').innerText()
  note('進度頁：' + pBody.replace(/\n+/g, ' | ').slice(0, 400))
  expect(pBody.includes(S.donorA.name)).toBe(false)
  expect(pBody.includes(S.donorB.name)).toBe(false)
  await expect(page.getByText(/還有 \d+ 天/)).toBeVisible()
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-31-progress-locked.png'), fullPage: true })
}, { page })

await run.step('R-32', '驚喜模式：鎖定中無法關閉；API 直接 PATCH 關閉／刪品項被拒', async ({ note }) => {
  need('listB')
  const cur = await wl(S.api, S.listB)
  const r = await S.api.patch(`wishlists/${S.listB}`, { data: { surprise_mode: false, expected_updated_at: cur.wishlist.updated_at } })
  note(`API PATCH surprise_mode=false → ${r.status()} ${(await r.text()).slice(0, 160)}`)
  const r2 = await S.api.delete(`items/${S.itemsB['派對帽']}?force=true`)
  note(`API DELETE 有認領品項 force → ${r2.status()} ${(await r2.text()).slice(0, 160)}`)
  expect([400, 403, 409, 422]).toContain(r.status())
}, { page })

await run.step('R-33', '驚喜解鎖（時間快轉）：活動日改為昨天 → 進度頁揭曉捐助者', async ({ note }) => {
  need('listB')
  sql(`UPDATE wishlists SET event_date = current_date - 1 WHERE id = '${S.listB}'`)
  note('平台外：UPDATE wishlists SET event_date = 昨天（模擬活動日過後）')
  await page.goto(`/lists/${S.listB}/progress`); await hydrated(page)
  await expect(page.getByText(S.donorA.name).first()).toBeVisible({ timeout: 15000 })
  const body = await page.locator('main').innerText()
  note('解鎖後是否出現捐助者名字 A = ' + body.includes(S.donorA.name) + '、B = ' + body.includes(S.donorB.name))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-33-unlocked.png'), fullPage: true })
  expect(body.includes(S.donorA.name) && body.includes(S.donorB.name)).toBe(true)
  await expect(page.getByText(/還有 \d+ 天/)).toHaveCount(0)
}, { page })

// ---------- H. 好友與可見度 ----------
await run.step('R-34', '建立好友：用 Email 送邀請、對方接受；用 @handle 送邀請、婉拒；邀請連結', async ({ note }) => {
  S.fr1 = await registerApi({ name: `好友小美${tag.slice(0, 3)}`, tag: 'fr' })
  S.fr2 = await registerApi({ name: `摯友小強${tag.slice(0, 3)}`, tag: 'fr' })
  S.stranger = await registerApi({ name: `路人${tag.slice(0, 3)}`, tag: 'str' })
  S.fr3 = await registerApi({ name: `邀請連結好友${tag.slice(0, 3)}`, tag: 'fr' })
  S.h3 = `h${tag}z`
  const hp = await S.fr3.api.patch('me', { data: { handle: S.h3 } }); expect(hp.ok()).toBe(true)
  note('平台外：以 API 註冊好友 ×3、路人 ×1（路人與好友3 另設 handle）')
  await page.goto('/friends'); await hydrated(page)
  await expect(page.getByText('還沒有好友，用上面的方式邀請朋友吧。')).toBeVisible()
  await page.getByLabel('對方的 Email 或帳號代號').fill(S.fr1.email)
  await page.getByRole('button', { name: '送出邀請' }).click()
  await expect(page.getByText('若對方有帳號，已送出邀請')).toBeVisible()
  await expect(page.getByRole('heading', { name: '已送出的邀請' })).toBeVisible()
  note('Email 邀請已送出，出現「已送出的邀請」')
  await page.getByLabel('對方的 Email 或帳號代號').fill('@' + S.h3)
  await page.getByRole('button', { name: '送出邀請' }).click()
  await expect(page.locator('.c-row', { hasText: S.fr3.name })).toBeVisible()
  note('@handle 邀請已送出（含 @ 前綴被去除）')
  await page.getByLabel('對方的 Email 或帳號代號').fill(S.fr2.email)
  await page.getByRole('button', { name: '送出邀請' }).click()
  await expect(page.locator('.c-row', { hasText: S.fr2.name })).toBeVisible()
  // 對方接受（fr1、fr2）/ 婉拒（fr3）
  for (const [u, act] of [[S.fr1, '接受'], [S.fr2, '接受'], [S.fr3, '婉拒']]) {
    const p = await newPage({ user: u })
    try {
      await p.goto('/friends'); await hydrated(p)
      const row = p.locator('.c-row', { hasText: owner.name })
      await expect(row).toBeVisible()
      await row.getByRole('button', { name: act }).click()
      await p.waitForTimeout(500)
    } finally { await p.context().close() }
  }
  await page.reload(); await hydrated(page)
  await expect(page.getByRole('heading', { name: '我的好友（2）' })).toBeVisible()
  note('接受後「我的好友（2）」；婉拒者不在名單，已送出邀請已清空 = ' + (await page.getByRole('heading', { name: '已送出的邀請' }).count() === 0))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-34-friends.png'), fullPage: true })
}, { page })

await run.step('R-35', '邀請連結：產生 → 顯示網址與 QR → 對方開連結成為好友；重新產生使舊連結失效；撤銷', async ({ note }) => {
  await page.goto('/friends'); await hydrated(page)
  await page.getByRole('button', { name: '產生邀請連結' }).click()
  const inv1 = await page.getByLabel('邀請網址').inputValue(); note('邀請網址：' + inv1)
  await expect(page.locator('#fr-invite').locator('svg, canvas, img').first()).toBeVisible()
  await page.getByRole('button', { name: '複製網址' }).click()
  await expect(page.getByRole('button', { name: '已複製' })).toBeVisible()
  await page.getByRole('button', { name: '重新產生' }).click()
  await expect(page.getByLabel('邀請網址')).not.toHaveValue(inv1)
  const inv2 = await page.getByLabel('邀請網址').inputValue()
  // 舊連結
  const pOld = await newPage({ user: S.stranger })
  await pOld.goto(inv1.replace(/^https?:\/\/[^/]+/, '')); await hydrated(pOld)
  await expect(pOld.getByText(/失效|撤銷/).first()).toBeVisible({ timeout: 10000 })
  const oldTxt = await pOld.locator('main').innerText(); note('舊連結頁面：' + oldTxt.replace(/\n+/g, ' | ').slice(0, 120))
  await pOld.context().close()
  expect(oldTxt).toMatch(/失效|撤銷/)
  // 新連結：好友3 先婉拒過，改用連結成為好友
  const p3 = await newPage({ user: S.fr3 })
  await p3.goto(inv2.replace(/^https?:\/\/[^/]+/, '')); await hydrated(p3)
  await p3.getByRole('button', { name: '成為好友' }).click()
  await p3.waitForTimeout(800)
  note('好友3 接受後頁面：' + (await p3.locator('main').innerText()).replace(/\n+/g, ' | ').slice(0, 120))
  await p3.context().close()
  await page.reload(); await hydrated(page)
  await expect(page.getByRole('heading', { name: '我的好友（3）' })).toBeVisible()
  note('透過連結成為好友後：我的好友（3）')
}, { page })

await run.step('R-36', '清單 C（所有好友）：好友看得到、路人看到「無權限」並可送好友申請、匿名要登入', async ({ note }) => {
  S.titleC = `好友限定清單 ${tag}`
  S.listC = await uiCreateList(page, { title: S.titleC, vis: '所有好友' })
  const d = await uiAddItem(page, { title: '好友限定禮', qty: 1 }); await saveItem(page, d); await expect(itemCard(page, '好友限定禮')).toBeVisible()
  const { dlg, slug } = await publish(page); S.slugC = slug
  await expect(dlg.getByText('附上好友邀請')).toBeVisible()
  note('friends 清單分享面板出現「附上好友邀請」；說明：' + await dlg.getByText(/對方接受邀請後成為你的好友/).innerText())
  await closeSheet(dlg)
  const view = async (u, expectText) => {
    const p = await newPage({ user: u })
    try { await p.goto(`/s/${S.slugC}`); await hydrated(p); await expect(p.getByRole('heading', { name: expectText })).toBeVisible({ timeout: 10000 }); return (await p.locator('main').innerText()).replace(/\n+/g, ' | ').slice(0, 200) }
    finally { await p.context().close() }
  }
  note('好友1：' + await view(S.fr1, S.titleC))
  note('路人（已登入非好友）：' + await view(S.stranger, '你沒有權限查看這份清單'))
  const anon = await newPage(); await anon.goto(`/s/${S.slugC}`); await hydrated(anon)
  await expect(anon.getByRole('heading', { name: '請先登入' })).toBeVisible(); note('匿名：請先登入')
  await anon.context().close()
  // 路人送好友申請（由清單頁）
  const ps = await newPage({ user: S.stranger })
  await ps.goto(`/s/${S.slugC}`); await hydrated(ps)
  await ps.getByRole('button', { name: new RegExp('加.*好友|送出.*邀請|好友') }).first().click()
  await expect(ps.getByRole('status').filter({ hasText: /送出|邀請/ })).toBeVisible()
  note('路人由清單頁送出好友申請 → ' + await ps.getByRole('status').filter({ hasText: /送出|邀請/ }).first().innerText())
  await ps.context().close()
  await page.goto('/friends'); await hydrated(page)
  await expect(page.locator('.c-row', { hasText: S.stranger.name }).getByRole('button', { name: '接受' })).toBeVisible()
  note('受捐者 /friends「收到的邀請」出現路人；先不接受（維持路人狀態）')
}, { page })

await run.step('R-37', '清單 D（摯友 selected）：UI 勾選好友 2；好友1 看不到、好友2 看得到；改勾選後即時生效', async ({ note }) => {
  S.titleD = `摯友限定清單 ${tag}`
  await page.goto('/lists/new'); await hydrated(page)
  await page.getByLabel(/清單名稱/).fill(S.titleD)
  await page.getByRole('radio', { name: /^摯友/ }).check()
  await expect(page.getByText('選擇摯友（已選 0 位）')).toBeVisible()
  await page.getByLabel(new RegExp(S.fr2.name)).check()
  await expect(page.getByText('選擇摯友（已選 1 位）')).toBeVisible()
  await page.getByRole('button', { name: '建立並新增品項' }).click()
  await page.waitForURL(/\/lists\/[0-9a-f-]+\/edit/); await hydrated(page)
  S.listD = page.url().match(/lists\/([0-9a-f-]+)\/edit/)[1]
  const d = await uiAddItem(page, { title: '摯友專屬禮', qty: 1 }); await saveItem(page, d); await expect(itemCard(page, '摯友專屬禮')).toBeVisible()
  const { dlg, slug } = await publish(page); S.slugD = slug
  note('selected 面板的邀請說明：' + (await dlg.getByText('附上好友邀請').count() ? '有「附上好友邀請」' : '無'))
  await closeSheet(dlg)
  const look = async (u) => { const p = await newPage({ user: u }); try { await p.goto(`/s/${S.slugD}`); await hydrated(p); await p.waitForTimeout(600); return (await p.locator('h1').first().innerText()) } finally { await p.context().close() } }
  const h1 = await look(S.fr1), h2 = await look(S.fr2)
  note(`好友1(未勾選) 看到：「${h1}」；好友2(已勾選) 看到：「${h2}」`)
  expect(h2).toBe(S.titleD); expect(h1).not.toBe(S.titleD)
  // 編輯：加勾好友 1
  await page.reload(); await hydrated(page)
  await page.getByText('清單設定', { exact: true }).click()
  await expect(page.getByLabel(new RegExp(S.fr2.name))).toBeChecked()
  await page.getByLabel(new RegExp(S.fr1.name)).check()
  await page.getByRole('button', { name: '儲存設定' }).click()
  await expect(page.getByText('已儲存')).toBeVisible()
  const h1b = await look(S.fr1); note('加勾好友1後，好友1 看到：「' + h1b + '」'); expect(h1b).toBe(S.titleD)
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-37-selected.png'), fullPage: true })
}, { page })

await run.step('R-38', '清單 E（私人）與可見度收緊：好友看到 404；公開→私人後訪客 404；探索頁不列出', async ({ note }) => {
  S.titleE = `私人清單 ${tag}`
  S.listE = await uiCreateList(page, { title: S.titleE, vis: '私人' })
  const d = await uiAddItem(page, { title: '私房禮', qty: 1 }); await saveItem(page, d); await expect(itemCard(page, '私房禮')).toBeVisible()
  const { dlg, slug } = await publish(page); S.slugE = slug; await closeSheet(dlg)
  const p = await newPage({ user: S.fr1 })
  const res = await p.goto(`/s/${S.slugE}`); await hydrated(p)
  await expect(p.getByRole('heading', { name: '找不到這份清單' })).toBeVisible(); note(`好友看私人清單 → 「找不到這份清單」(HTTP ${res.status()})`)
  await p.context().close()
  await page.goto(`/s/${S.slugE}`); await hydrated(page)
  await expect(page.getByRole('heading', { level: 1, name: S.titleE })).toBeVisible(); note('擁有者自己看得到')
  // 公開 A → 改私人 → 匿名 404；再改回公開
  await page.goto(`/lists/${S.listA}/edit`); await hydrated(page)
  await page.getByText('清單設定', { exact: true }).click()
  await page.getByRole('radio', { name: /^私人/ }).check()
  await page.getByRole('button', { name: '儲存設定' }).click(); await expect(page.getByText('已儲存')).toBeVisible()
  const anon = await newPage()
  const r = await anon.goto(`/s/${S.slugA}`); await hydrated(anon); await anon.waitForTimeout(500)
  note(`A 改私人後匿名開分享頁 → HTTP ${r.status()}，h1=「${await anon.locator('h1').first().innerText()}」`)
  await page.getByRole('radio', { name: /^公開/ }).check()
  await page.getByRole('button', { name: '儲存設定' }).click(); await expect(page.getByText('已儲存')).toBeVisible()
  await anon.reload(); await hydrated(anon)
  await expect(anon.getByRole('heading', { level: 1, name: S.titleA2 })).toBeVisible(); note('改回公開後匿名恢復可見')
  await anon.goto('/explore'); await hydrated(anon)
  const ex = await anon.locator('main').innerText(); note('探索頁是否列出 A = ' + ex.includes(S.titleA2) + '；是否列出私人 E = ' + ex.includes(S.titleE))
  expect(ex.includes(S.titleE)).toBe(false)
  await anon.context().close()
}, { page })

await run.step('R-38b', '清單（公開 A）被營運下架：受捐者在 dashboard／編輯頁看到下架原因、分享停用；訪客 410；恢復後正常', async ({ note }) => {
  need('listA')
  const hide = await S.staff.api.patch(`admin/wishlists/${S.listA}/moderation`, { data: { moderation_status: 'hidden', reason: '疑似詐騙（QA 測試）' } })
  note('平台外：營運（API）下架清單 A（公開）→ ' + hide.status())
  expect(hide.ok()).toBe(true)
  await page.goto('/dashboard'); await hydrated(page)
  const card = page.locator('.c-card', { hasText: S.titleA2 })
  await expect(card.getByRole('alert')).toContainText('已被下架')
  note('dashboard 卡片：' + (await card.innerText()).replace(/\n+/g, ' | '))
  await page.goto(`/lists/${S.listA}/edit`); await hydrated(page)
  await expect(page.getByRole('alert').first()).toContainText('已被下架')
  await expect(page.getByRole('button', { name: '分享', exact: true })).toBeDisabled()
  note('編輯頁：「分享」按鈕停用；訊息：' + await page.getByRole('alert').first().innerText())
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-38b-hidden.png') })
  const api = await apiCtx(); const pub = await api.get(`public/wishlists/${S.slugA}`)
  note('下架後公開 API → ' + pub.status()); expect(pub.status()).toBe(410)
  const an = await newPage(); const rs = await an.goto(`/s/${S.slugA}`); await hydrated(an)
  note(`訪客開分享頁 → HTTP ${rs.status()}，內容：` + (await an.locator('main').innerText()).replace(/\n+/g, ' | ').slice(0, 120)); await an.context().close()
  const unhide = await S.staff.api.patch(`admin/wishlists/${S.listA}/moderation`, { data: { moderation_status: 'ok' } })
  note('營運恢復 → ' + unhide.status())
  await page.reload(); await hydrated(page)
  await expect(page.getByRole('button', { name: '分享', exact: true })).toBeEnabled()
  const mails = (await listMail(owner.email)).map(m => m.subject)
  note('受捐者信箱主旨（含下架通知？）：' + mails.join(' | '))
  if (!mails.some(m => /下架|移除|檢舉/.test(m))) note('受捐者沒有收到「清單被下架」通知信，只能在 dashboard／編輯頁看到橫幅')
}, { page })

await run.step('R-39', '好友動態：好友登入 /friends 看到受捐者新清單；生日卡片', async ({ note }) => {
  const p = await newPage({ user: S.fr1 })
  try {
    await p.goto('/friends'); await hydrated(p)
    await expect(p.locator('.fr-feed')).toContainText(owner.name, { timeout: 10000 })
    const feed = (await p.locator('.fr-feed').innerText()).replace(/\n+/g, ' | ').slice(0, 400)
    note('好友動態：' + feed)
    await p.screenshot({ path: resolve(OUT, 'recipient', 'R-39-feed.png'), fullPage: true })
  } finally { await p.context().close() }
}, { page })

await run.step('R-40', '解除好友：確認對話框；對方立即失去好友限定清單存取、摯友名單移除', async ({ note }) => {
  await page.goto('/friends'); await hydrated(page)
  await page.locator('.c-row', { hasText: S.fr1.name }).getByRole('button', { name: '解除好友' }).click()
  const dlg = page.getByRole('dialog', { name: new RegExp('解除與 ' + S.fr1.name) })
  note('對話框：' + (await dlg.innerText()).replace(/\n+/g, ' | '))
  await dlg.getByRole('button', { name: '解除好友' }).click()
  await expect(page.getByRole('heading', { name: '我的好友（2）' })).toBeVisible()
  const p = await newPage({ user: S.fr1 }); await p.goto(`/s/${S.slugC}`); await hydrated(p)
  await expect(p.getByRole('heading', { name: '你沒有權限查看這份清單' })).toBeVisible({ timeout: 10000 }); note('好友1 立即失去 C 清單存取')
  await p.goto(`/s/${S.slugD}`); await hydrated(p); await p.waitForTimeout(500)
  note('好友1 開 D（摯友）→ h1=「' + await p.locator('h1').first().innerText() + '」')
  await p.context().close()
}, { page })

// ---------- I. 結束、封存、自動結束 ----------
S.titleF = `結束測試清單 ${tag}`
await run.step('R-41', '結束清單 → 分享頁顯示已結束、捐助者不能再認領 → 重新開放', async ({ note }) => {
  S.listF = await uiCreateList(page, { title: S.titleF, vis: '公開' })
  const d = await uiAddItem(page, { title: '結束測試品', qty: 2 }); await saveItem(page, d); await expect(itemCard(page, '結束測試品')).toBeVisible()
  const { dlg, slug } = await publish(page); S.slugF = slug; await closeSheet(dlg)
  S.itemF = (await wl(S.api, S.listF)).items[0].id
  await page.getByText('清單設定', { exact: true }).click()
  await page.getByRole('button', { name: '結束清單' }).click()
  await expect(page.getByText('清單已結束，品項無法修改。')).toBeVisible()
  for (const n of ['＋ 新增品項', '上移', '編輯']) await expect(page.getByRole('button', { name: n })).toHaveCount(0)
  note('結束後新增／排序／編輯／刪除按鈕全部消失')
  const anon = await newPage(); await anon.goto(`/s/${S.slugF}`); await hydrated(anon)
  await expect(anon.getByText('這份清單已結束')).toBeVisible().catch(() => {})
  note('分享頁橫幅：' + (await anon.locator('.g-banner').allInnerTexts()).join(' / '))
  await anon.context().close()
  const r = await S.donorA.api.post(`items/${S.itemF}/claims`, { data: { qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } })
  note(`結束後捐助者認捐 API → ${r.status()} ${(await r.text()).slice(0, 120)}`)
  expect(r.ok()).toBe(false)
  await page.getByRole('button', { name: '重新開放' }).click()
  await expect(page.getByRole('button', { name: '＋ 新增品項' })).toBeVisible()
  const r2 = await S.donorA.api.post(`items/${S.itemF}/claims`, { data: { qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } })
  note('重新開放後認捐 → ' + r2.status()); expect(r2.ok()).toBe(true)
}, { page })

await run.step('R-42', 'closes_at 自動結束（設定 → 時間快轉 → 等背景 job）', async ({ note }) => {
  S.titleG = `自動結束清單 ${tag}`
  S.listG = await uiCreateList(page, { title: S.titleG, vis: '公開', closes: localIn(2) })
  const d = await uiAddItem(page, { title: '自動結束品', qty: 3 }); await saveItem(page, d); await expect(itemCard(page, '自動結束品')).toBeVisible()
  const { dlg, slug } = await publish(page); S.slugG = slug; await closeSheet(dlg)
  S.itemG = (await wl(S.api, S.listG)).items[0].id
  await claimApi(S.donorB, S.itemG, { qty: 1 })
  const anon = await newPage(); await anon.goto(`/s/${S.slugG}`); await hydrated(anon)
  await expect(anon.getByText(/結束・/)).toBeVisible(); note('分享頁顯示結束時間：' + await anon.getByText(/結束・/).innerText())
  await anon.context().close()
  sql(`UPDATE wishlists SET closes_at = now() - interval '1 minute' WHERE id = '${S.listG}'`)
  S.closeAt = Date.now()
  note('平台外：UPDATE closes_at = 一分鐘前（模擬時間流逝）；背景 close_due 每 5 分鐘一輪，稍後在 R-47 回頭驗證狀態轉換')
  const r = await S.donorA.api.post(`items/${S.itemG}/claims`, { data: { qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } })
  note(`closes_at 已過、job 尚未跑：捐助者認捐 → ${r.status()} ${(await r.text()).slice(0, 100)}`)
  expect(r.ok()).toBe(false)
  await page.goto('/dashboard'); await hydrated(page)
  note('job 前 dashboard 卡片狀態：' + (await page.locator('.c-card', { hasText: S.titleG }).innerText()).replace(/\n+/g, ' | '))
}, { page })

await run.step('R-43', '封存清單（dashboard）→ 已封存列表 → 公開頁 404 → 取消封存（回到已結束）', async ({ note }) => {
  await page.goto('/dashboard'); await hydrated(page)
  await page.locator('.c-card', { hasText: S.titleF }).getByRole('button', { name: '封存' }).click()
  const dlg = page.getByRole('dialog', { name: '封存清單' })
  note('封存警語：' + await dlg.locator('p').first().innerText())
  await dlg.getByRole('button', { name: '確認封存' }).click()
  await expect(page.locator('.c-card', { hasText: S.titleF })).toHaveCount(0)
  const anon = await newPage(); const r = await anon.goto(`/s/${S.slugF}`); await hydrated(anon)
  note('封存後匿名開分享頁 HTTP ' + r.status()); expect(r.status()).toBe(404); await anon.context().close()
  await page.getByRole('button', { name: '已封存的清單' }).click()
  await expect(page.locator('.c-card', { hasText: S.titleF })).toBeVisible()
  await page.locator('.c-card', { hasText: S.titleF }).getByRole('button', { name: '取消封存' }).click()
  await expect(page.locator('.c-card', { hasText: S.titleF })).toHaveCount(0)
  await page.getByRole('button', { name: '← 返回我的清單' }).click()
  const card = page.locator('.c-card', { hasText: S.titleF })
  await expect(card).toBeVisible(); note('取消封存後狀態：' + (await card.locator('.c-badge').first().innerText()))
  await expect(card).toContainText('已結束')
}, { page })

// ---------- J. 設定、個人頁 ----------
await run.step('R-44', '設定頁：顯示名稱修改', async ({ note }) => {
  await page.goto('/settings'); await hydrated(page)
  const nn = owner.name + '改'
  const form = page.locator('form', { has: page.getByLabel('顯示名稱') })
  await form.getByLabel('顯示名稱').fill(nn)
  await form.getByRole('button', { name: '儲存' }).click()
  await expect(form.getByText('已儲存')).toBeVisible()
  owner.name = nn
  await page.reload(); await hydrated(page)
  await expect(page.getByLabel('顯示名稱')).toHaveValue(nn)
  await page.goto(`/s/${S.slugA}`); await hydrated(page)
  await expect(page.getByText(nn).first()).toBeVisible()
  note('改名後分享頁「建立者」即時反映')
}, { page })

await run.step('R-45', '個人頁設定：帳號代號（格式／重複）、自我介紹、生日、生日可見度、預設送禮紀錄可見度', async ({ note }) => {
  await page.goto('/settings'); await hydrated(page)
  const input = page.getByLabel('帳號代號（個人頁網址，可留空）')
  const form = page.locator('form', { has: input })
  await input.fill('AB')
  await expect(page.getByText('小寫英文、數字、底線，3–30 字')).toBeVisible()
  await expect(form.getByRole('button', { name: '儲存' })).toBeDisabled()
  note('不合法代號：顯示提示並停用儲存')
  await input.fill(S.h3)
  await form.getByRole('button', { name: '儲存' }).click()
  await expect(form.getByText('這個帳號代號已被使用，換一個試試')).toBeVisible(); note('重複代號：提示已被使用')
  S.handle = `rcp${tag}`
  await input.fill(S.handle)
  await form.getByLabel(/自我介紹/).fill('熱愛手作的新手媽媽\n第二行')
  await form.getByLabel('月', { exact: true }).selectOption('07')
  await form.getByLabel('日', { exact: true }).selectOption('15')
  await form.getByLabel('生日誰看得到').selectOption('friends')
  await form.getByLabel('送禮紀錄預設誰看得到').selectOption('public')
  await form.getByRole('button', { name: '儲存' }).click()
  await expect(form.getByText('已儲存')).toBeVisible()
  await expect(form.getByRole('link', { name: new RegExp('/u/' + S.handle + '$') })).toBeVisible()
  await page.reload(); await hydrated(page)
  await expect(page.getByLabel(/自我介紹/)).toHaveValue('熱愛手作的新手媽媽\n第二行')
  await expect(page.getByLabel('月', { exact: true })).toHaveValue('07')
  note('重整後設定仍在；2/30 校正：')
  await page.getByLabel('月', { exact: true }).selectOption('02')
  const days = await page.getByLabel('日', { exact: true }).locator('option').count(); note(`2 月日選單選項數 = ${days - 1}`)
  await page.getByLabel('月', { exact: true }).selectOption('07')
}, { page })

await run.step('R-46', '個人頁 /u/[handle]：本人視角、好友視角、陌生人視角（加好友）、匿名視角；只列公開清單', async ({ note }) => {
  need('handle')
  await page.goto(`/u/${S.handle}`); await hydrated(page)
  await expect(page.getByRole('heading', { name: owner.name }).first()).toBeVisible()
  await expect(page.getByRole('link', { name: '編輯個人設定' }).or(page.getByRole('link', { name: /設定/ })).first()).toBeVisible()
  const self = (await page.locator('main').innerText()).replace(/\n+/g, ' | ')
  note('本人視角：' + self.slice(0, 420))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-46-profile-self.png'), fullPage: true })
  const view = async (u, label) => { const p = await newPage(u ? { user: u } : {}); try { await p.goto(`/u/${S.handle}`); await hydrated(p); await p.waitForTimeout(600); const t = (await p.locator('main').innerText()).replace(/\n+/g, ' | '); note(`${label}視角：` + t.slice(0, 420)); return t } finally { await p.context().close() } }
  const fr2 = await view(S.fr2, '好友(摯友2)')
  const str = await view(S.stranger, '陌生人(已送出申請)')
  const anon = await view(null, '匿名')
  for (const t of [fr2, str, anon]) { expect(t.includes(S.titleE)).toBe(false) }
  note('私人清單 E 在任何視角都不出現（已驗證）')
  note('好友視角是否看到好友限定清單 C = ' + fr2.includes(S.titleC) + '；匿名是否看到 C = ' + anon.includes(S.titleC) + '；匿名是否看到公開 A = ' + anon.includes(S.titleA2))
  note('匿名／陌生人視角生日顯示：' + (/7\/15|07-15|7 月 15|生日/.test(anon) ? '有' : '無') + '（生日可見度=好友）')
}, { page })

await run.step('R-47', '通知偏好與生日清單通知：設定頁開關、清單開關（填生日後可開）', async ({ note }) => {
  await page.goto('/settings'); await hydrated(page)
  const sw = page.getByLabel('社交與生日 Email')
  await expect(sw).toBeChecked()
  const [r] = await Promise.all([page.waitForResponse(x => x.request().method() === 'PATCH' && /\/me$/.test(x.url())), sw.uncheck()])
  note('關閉生日通知信 PATCH ' + r.status())
  await page.reload(); await hydrated(page)
  await expect(page.getByLabel('社交與生日 Email')).not.toBeChecked()
  await page.getByLabel('社交與生日 Email').check(); await page.waitForTimeout(500)
  note('再開啟')
  // 清單層級生日通知
  S.titleH = `生日清單 ${tag}`
  S.listH = await uiCreateList(page, { title: S.titleH, vis: '所有好友', bday: true })
  await page.getByText('清單設定', { exact: true }).click()
  await expect(page.getByLabel(/生日前 7 天通知/)).toBeChecked()
  note('填生日後可開啟清單生日通知，編輯頁顯示已開啟')
  await page.getByRole('radio', { name: /^私人/ }).check()
  await expect(page.getByLabel(/生日前 7 天通知/)).toBeDisabled()
  note('改私人 → 開關自動停用：' + await page.getByText('私人清單不會通知任何人，請先改成其他可見度。').innerText())
  await page.getByRole('radio', { name: /^所有好友/ }).check()
}, { page })

await run.step('R-48', '密碼與登入裝置：變更密碼（錯誤舊密碼被拒）→ 其他裝置登出；登入裝置列表', async ({ note }) => {
  const other = await newPage()
  await uiLogin(other, owner.email); await hydrated(other)
  await page.goto('/settings'); await hydrated(page)
  await expect(page.locator('#sessions-title').locator('xpath=..').locator('li').nth(1)).toBeVisible({ timeout: 10000 })
  const rows = await page.locator('#sessions-title').locator('xpath=..').locator('li').count()
  note('登入裝置數 = ' + rows)
  expect(rows).toBeGreaterThanOrEqual(2)
  const pwSec = page.locator('section', { has: page.getByRole('heading', { name: '變更密碼' }) })
  await pwSec.getByLabel('目前密碼').fill('wrong-current-pw')
  await pwSec.getByLabel('新密碼').fill('Brand-New-pass-9')
  await pwSec.getByRole('button', { name: '變更密碼' }).click()
  note('舊密碼錯誤：' + await pwSec.getByRole('alert').innerText())
  await pwSec.getByLabel('目前密碼').fill(owner.password)
  await pwSec.getByRole('button', { name: '變更密碼' }).click()
  await expect(pwSec.getByText('密碼已變更，其他裝置已登出')).toBeVisible()
  owner.password = 'Brand-New-pass-9'
  await other.goto('/dashboard'); await other.waitForTimeout(1200)
  note('其他裝置重新載入 dashboard 後網址：' + other.url())
  expect(other.url()).toContain('/login')
  await other.context().close()
  S.api = await ownerLogin(owner.email, owner.password)
}, { page })

await run.step('R-49', '資料匯出：下載 JSON，內容含個人資料、清單、品項', async ({ note }) => {
  await page.goto('/settings'); await hydrated(page)
  const [dl] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: '匯出我的資料' }).click()])
  const f = await dl.path(); const data = JSON.parse(readFileSync(f, 'utf8'))
  note(`檔名 ${dl.suggestedFilename()}；頂層欄位：${Object.keys(data).join(',')}`)
  expect(data.user.email).toBe(owner.email)
  const titles = data.wishlists.map(w => w.title)
  note('匯出清單數 = ' + titles.length + '；含 A = ' + titles.includes(S.titleA2))
  expect(titles).toContain(S.titleA2)
  const w0 = data.wishlists.find(w => w.title === S.titleA2)
  note('清單 A 匯出欄位：' + Object.keys(w0).join(',') + '；items = ' + (w0.items?.length ?? '(無)'))
  const dumpHasShip = JSON.stringify(data).includes('和平東路')
  note('匯出是否含收件地址明文 = ' + dumpHasShip + '；是否含認捐者資訊 = ' + JSON.stringify(data).includes(S.donorA.name))
}, { page })

// ---------- K. 登出登入、忘記密碼 ----------
await run.step('R-50', '登出 → 受限頁被擋 → 錯誤密碼不洩漏帳號存在 → 正確登入 → 回到 redirect', async ({ note }) => {
  await uiLogout(page)
  await page.goto(`/lists/${S.listA}/edit`); await expect(page).toHaveURL(/\/login\?redirect=/)
  note('登出後進編輯頁導向：' + page.url())
  await page.getByLabel('Email').fill(owner.email)
  await page.getByLabel('密碼').fill('totally-wrong-pw')
  await page.getByRole('button', { name: '登入', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveText('帳號或密碼錯誤')
  await page.getByLabel('Email').fill(`nobody-${tag}@example.com`)
  await page.getByRole('button', { name: '登入', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveText('帳號或密碼錯誤'); note('不存在帳號訊息相同')
  await page.getByLabel('Email').fill(owner.email)
  await page.getByLabel('密碼').fill('Brand-New-pass-9')
  await page.getByRole('button', { name: '登入', exact: true }).click()
  await page.waitForURL(u => !u.pathname.startsWith('/login'), { timeout: 15000 })
  note('登入後導向：' + page.url()); expect(page.url()).toContain(`/lists/${S.listA}/edit`)
  const old = await (await apiCtx()).post('auth/login', { data: { email: owner.email, password: PASSWORD } })
  note('舊密碼登入 API → ' + old.status()); expect(old.status()).toBe(401)
}, { page })

await run.step('R-51', '忘記密碼（UI）：寄 OTP → 輸入新密碼 → 以新密碼登入；舊 session 失效', async ({ note }) => {
  await uiLogout(page)
  resetOtpLimits()
  await page.getByRole('link', { name: '忘記密碼' }).click()
  await page.waitForURL(/forgot-password/); await hydrated(page)
  await page.getByLabel('Email').fill(owner.email)
  const t0 = Date.now() - 1000
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  await expect(page.getByText(/已寄出驗證碼/)).toBeVisible()
  const m = await waitForMail(owner.email, { after: t0, subject: /密碼|重設/ })
  note(`平台外：從 Mailpit 取 OTP（主旨「${m.subject}」）`)
  await page.getByLabel('第 1 碼').fill(otpFrom(m.text))
  await page.getByLabel('新密碼').fill('short')
  await page.getByRole('button', { name: '重設密碼' }).click()
  note('新密碼太短：' + await page.getByRole('alert').innerText())
  await page.getByLabel('新密碼').fill('Reset-pass-77x')
  await page.getByRole('button', { name: '重設密碼' }).click()
  await expect(page).toHaveURL(/\/login\?reset=1/)
  await expect(page.getByText('密碼已重設，請用新密碼登入')).toBeVisible()
  owner.password = 'Reset-pass-77x'
  await uiLogin(page, owner.email, owner.password); await hydrated(page)
  await expect(page).toHaveURL(/\/dashboard/)
  note('新密碼登入成功')
  S.api = await ownerLogin(owner.email, owner.password)
  const unknown = await (await apiCtx()).post('auth/password/reset/request', { data: { email: `ghost-${tag}@example.com` } })
  note('不存在 Email 要求重設 API → ' + unknown.status() + '（與存在時是否相同見 R-51 頁面訊息「若帳號存在」）')
}, { page })

// ---------- L. 權限與安全 ----------
await run.step('R-52', '他人清單的編輯頁／進度頁：受捐者不能開別人的清單', async ({ note }) => {
  S.api = await ownerLogin(owner.email, owner.password)
  const other = await registerApi({ name: `別人${tag.slice(0, 3)}`, tag: 'oth' })
  const lr = await other.api.post('wishlists', { data: { type: 'personal', title: `別人的清單 ${tag}`, visibility: 'public' } })
  const otherList = (await lr.json()).id
  const p = await newPage({ user: owner.__noCookie ? undefined : undefined })
  await uiLogin(p, owner.email, owner.password)
  for (const path of [`/lists/${otherList}/edit`, `/lists/${otherList}/progress`]) {
    await p.goto(path); await hydrated(p); await p.waitForTimeout(800)
    const t = (await p.locator('main').innerText()).replace(/\n+/g, ' | ').slice(0, 160)
    note(`${path.replace(otherList, ':別人id')} → ${t}`)
    expect(t).not.toContain(`別人的清單 ${tag}`)
  }
  const r = await S.api.patch(`wishlists/${otherList}`, { data: { title: '駭' } })
  note('API 直接 PATCH 別人清單 → ' + r.status()); expect([403, 404]).toContain(r.status())
  const r2 = await S.api.delete(`wishlists/${otherList}`)
  note('API DELETE 別人清單 → ' + r2.status()); expect([403, 404]).toContain(r2.status())
  await p.context().close()
}, { page })

await run.step('R-53', '主題與語系：切換語系後創建者頁面不壞（en）', async ({ note }) => {
  await page.goto('/dashboard'); await hydrated(page)
  const sel = page.locator('.c-head select, .c-head button').filter({ hasText: /語言|中文|EN|English/i }).first()
  const before = await page.locator('h1').first().innerText()
  const sw = page.locator('.c-head').getByRole('combobox').first()
  note('頁首語言選單選項：' + (await sw.locator('option').allInnerTexts()).join(' / ') + '；切換前 h1=' + before)
  await sw.selectOption({ label: 'English' })
  await page.waitForTimeout(1000)
  const en = await page.locator('h1').first().innerText()
  note('切換 English 後 h1=' + en + '；分享按鈕：' + (await page.locator('main').innerText()).replace(/\n+/g, ' | ').slice(0, 160))
  await page.goto(`/lists/${S.listA}/edit`); await hydrated(page)
  note('English 編輯頁按鈕：' + (await page.getByRole('button').allInnerTexts()).slice(0, 12).join(' | '))
  await page.screenshot({ path: resolve(OUT, 'recipient', 'R-53-en.png') })
  await page.goto('/dashboard'); await hydrated(page)
  await page.locator('.c-head').getByRole('combobox').first().selectOption({ label: '繁體中文' })
  await page.waitForTimeout(800)
  expect(await page.locator('h1').first().innerText()).toBe(before)
  expect(en).not.toBe(before)
}, { page })

// ---------- M. 帳號刪除（拋棄帳號） ----------
await run.step('R-54', '刪除帳號（拋棄帳號，UI 註冊）：確認字、密碼錯誤擋下；刪除後清單下架、無法登入、可重新用同 Email 註冊', async ({ note }) => {
  const dp = await newPage()
  const del = { name: `拋棄帳號${tag.slice(0, 3)}`, email: `flow-recipient-del-${tag}@example.com`, password: PASSWORD }
  try {
    const t0 = await uiRegister(dp, del)
    const m = await waitForMail(del.email, { after: t0 }); await dp.getByLabel('第 1 碼').fill(otpFrom(m.text))
    await expect(dp).toHaveURL(/\/dashboard$/, { timeout: 15000 })
    const lid = await uiCreateList(dp, { title: `拋棄清單 ${tag}`, vis: '公開' })
    const d = await uiAddItem(dp, { title: '拋棄品項', qty: 1 }); await saveItem(dp, d); await expect(itemCard(dp, '拋棄品項')).toBeVisible()
    const { dlg, slug } = await publish(dp); await closeSheet(dlg)
    S.delSlug = slug
    await dp.goto('/settings'); await hydrated(dp)
    await dp.getByRole('button', { name: '刪除我的帳號' }).click()
    const dd = dp.getByRole('dialog', { name: '確定要刪除帳號嗎？' })
    const ok = dd.getByRole('button', { name: '永久刪除' })
    await expect(ok).toBeDisabled()
    await dd.getByLabel('請輸入「刪除」以確認').fill('刪除')
    await expect(ok).toBeDisabled(); note('只輸入確認字、未填密碼：永久刪除停用')
    await dd.getByLabel('目前密碼', { exact: true }).fill('wrong-password')
    await ok.click()
    await expect(dd.getByText('密碼不正確')).toBeVisible(); note('密碼錯誤被擋')
    await dp.screenshot({ path: resolve(OUT, 'recipient', 'R-54-delete-dialog.png') })
    await dd.getByLabel('目前密碼', { exact: true }).fill(del.password)
    await ok.click()
    await expect(dp).toHaveURL(/\/$/, { timeout: 15000 })
    note('刪除後導向首頁')
    const a = await apiCtx(); const lr = await a.post('auth/login', { data: { email: del.email, password: del.password } })
    note('刪除後登入 API → ' + lr.status()); expect(lr.status()).toBe(401)
    const pub = await (await apiCtx()).get(`public/wishlists/${S.delSlug}`)
    note('刪除後公開清單 API → ' + pub.status()); expect(pub.status()).toBe(404)
    const rem = sql(`SELECT coalesce(email,'(null)') || ' / ' || coalesce(display_name,'(null)') FROM users WHERE email = '${q(del.email)}' OR id = (SELECT owner_id FROM wishlists WHERE id = '${lid}')`)
    note('平台外：DB 檢查該使用者列的 email／display_name = ' + rem)
    const mails = await listMail(del.email); note('刪除帳號後該 Email 信件主旨：' + mails.map(x => x.subject).join(' | '))
  } finally { await dp.context().close() }
}, { page })

await run.step('R-55', '刪除帳號被擋的情境：受捐者帳號有未結案眾籌（他人贊助中點數）時能否刪除', async ({ note }) => {
  // 新帳號：建立含眾籌品項的清單，由捐助者贊助後（未達標）嘗試刪除帳號
  const o2 = await registerApi({ name: `眾籌受捐者${tag.slice(0, 3)}`, tag: 'cfo' })
  const lr = await o2.api.post('wishlists', { data: { type: 'personal', title: `眾籌刪帳 ${tag}`, visibility: 'public' } })
  const lid = (await lr.json()).id
  const sa = await o2.api.put(`wishlists/${lid}/shipping-address`, { data: { recipient_name: '王小明', phone: '0912345678', address: '台北市大安區和平東路二段 1 號 3 樓' } })
  const ir = await o2.api.post(`wishlists/${lid}/items`, { data: { title: '眾籌品', funding_mode: 'crowdfund', target_points: 5000, funding_deadline: new Date(Date.now() + 10 * 864e5).toISOString(), priority: 'medium' } })
  const iid = (await ir.json()).id
  const cur = await (await o2.api.get(`wishlists/${lid}`)).json()
  const pr = await o2.api.patch(`wishlists/${lid}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } })
  note(`平台外：API 建立眾籌清單（shipping ${sa.status()}, item ${ir.status()}, publish ${pr.status()}）`)
  const g3 = await S.staff.api.post('admin/wallets/grants', { data: { user_id: await meId(S.donorA.api), points: 1000, reason: 'QA 受捐者流程（刪帳情境）' }, headers: { 'Idempotency-Key': randomUUID() } })
  note('營運（API）再發 1000 點給捐助者 A → ' + g3.status())
  const pl = await S.donorA.api.post(`items/${iid}/contributions`, { data: { points: 800 }, headers: { 'Idempotency-Key': randomUUID() } })
  note('捐助者贊助 800 點 → ' + pl.status() + ' ' + (await pl.text()).slice(0, 200))
  const p = await newPage({ user: o2 })
  try {
    await p.goto('/settings'); await hydrated(p)
    await p.getByRole('button', { name: '刪除我的帳號' }).click()
    const dd = p.getByRole('dialog', { name: '確定要刪除帳號嗎？' })
    await dd.getByLabel('請輸入「刪除」以確認').fill('刪除')
    await dd.getByLabel('目前密碼', { exact: true }).fill(o2.password)
    await dd.getByRole('button', { name: '永久刪除' }).click()
    await p.waitForTimeout(2500)
    const url = p.url()
    const alert = await dd.getByRole('alert').allInnerTexts().catch(() => [])
    note(`嘗試刪除 → 目前網址 ${url}；對話框訊息 ${JSON.stringify(alert)}`)
    const me = await o2.api.get('me'); note('刪除後 /me → ' + me.status())
    const contrib = sql(`SELECT status || ' ' || points FROM contributions WHERE item_id = '${iid}'`)
    note('平台外：該贊助的 DB 狀態 = ' + contrib)
    const bal = sql(`SELECT balance FROM point_wallets WHERE user_id = '${await meId(S.donorA.api)}'`)
    note('捐助者 A 錢包餘額 = ' + bal)
    await p.screenshot({ path: resolve(OUT, 'recipient', 'R-55-delete-crowd.png') })
  } finally { await p.context().close() }
}, { page })

// ---------- N. 回頭驗證 closes_at 自動結束 ----------
await run.step('R-56', 'closes_at 自動結束：背景 job 轉為已結束，dashboard／分享頁／編輯頁一致', async ({ note }) => {
  need('listG')
  const waitMs = Math.max(0, S.closeAt + 6 * 60e3 - Date.now())
  note(`距快轉已過 ${Math.round((Date.now() - S.closeAt) / 1000)} 秒，最多再等 ${Math.round(waitMs / 1000)} 秒`)
  const deadline = Date.now() + waitMs
  let st
  while (Date.now() < deadline) { st = sql(`SELECT status FROM wishlists WHERE id = '${S.listG}'`); if (st === 'closed') break; await sleep(10000) }
  note('平台外：DB wishlists.status = ' + st + '（總計 ' + Math.round((Date.now() - S.closeAt) / 1000) + ' 秒）')
  if (st !== 'closed') throw new Error('背景 close_due 超過 6 分鐘仍未把清單轉為 closed')
  await page.goto('/dashboard'); await hydrated(page)
  const card = page.locator('.c-card', { hasText: S.titleG })
  await expect(card).toContainText('已結束'); note('dashboard 顯示已結束')
  const anon = await newPage(); await anon.goto(`/s/${S.slugG}`); await hydrated(anon)
  note('分享頁橫幅：' + (await anon.locator('.g-banner').allInnerTexts()).join(' / ')); await expect(anon.locator('.g-banner', { hasText: /已結束/ })).toBeVisible()
  await anon.context().close()
  await page.goto(`/lists/${S.listG}/edit`); await hydrated(page)
  await expect(page.getByText('清單已結束，品項無法修改。')).toBeVisible()
  const mails = await listMail(owner.email); note('受捐者信箱主旨（全部）：' + mails.map(m => m.subject).join(' | '))
}, { page })

await run.step('R-57', '受捐者收到的通知信總覽（Mailpit）', async ({ note }) => {
  const mails = await listMail(owner.email)
  note(`受捐者信箱共 ${mails.length} 封：` + mails.map(m => m.subject).join(' | '))
  const kinds = sql(`SELECT kind || ':' || status || ':' || count(*) FROM notifications WHERE user_id = (SELECT id FROM users WHERE email='${q(owner.email)}') GROUP BY kind, status ORDER BY 1`)
  note('平台外：notifications 佇列（受捐者）：' + kinds.replace(/\n/g, ' ; '))
  const claimMails = mails.filter(m => /認領|認捐|有人|送禮/.test(m.subject)).length
  const settingsTxt = await (async () => { await page.goto('/settings'); await hydrated(page); return page.locator('main').innerText() })()
  note('設定頁「通知」區是否有「有人捐助時寄 Email」開關 = ' + /有人捐助|新捐助/.test(settingsTxt))
  if (!claimMails) throw new Gap('有人認捐／贊助後，受捐者完全不會收到通知信（D58 停用 claim.created，04 資料契約 §認領有明載）；但 PRD US-38／US-12 與 03 線框 5.28「有人捐助時寄 Email，第一筆即時、之後每日彙整，可在 /settings 關閉」仍寫著會寄 → 文件不一致；佇列：' + kinds.replace(/\n/g, ' ; '))
}, { page })

// ---------- O. 本環境無法驗證（⛔） ----------
await run.step('R-58', 'Google／LINE 第三方登入（點擊按鈕觀察）', async ({ note }) => {
  const p = await newPage()
  try {
    await p.goto('/login'); await hydrated(p)
    await p.getByRole('button', { name: '使用 Google 繼續' }).click()
    await p.waitForTimeout(2500)
    note('點 Google 後網址：' + p.url() + '；畫面訊息：' + (await p.getByRole('alert').allInnerTexts()).join(' / '))
  } finally { await p.context().close() }
  throw new Blocked('沒有 Google／LINE OAuth 憑證與外部供應商，無法完成授權往返（設定頁「綁定 Google／LINE」同理）')
}, { page })
await run.step('R-59', 'LINE 內建瀏覽器開啟分享連結／openExternalBrowser 實際跳轉', async () => { throw new Blocked('需真機 LINE app；僅驗證了連結參數（R-15）') })
await run.step('R-60', 'Turnstile 人機驗證（多次失敗後登入／註冊／忘記密碼）', async () => { throw new Blocked('Turnstile 需 Cloudflare 站台金鑰，dev 環境不啟用，無法觸發 CAPTCHA_REQUIRED 的真實挑戰') })
await run.step('R-61', '清單改可見度／封存後 Cloudflare 快取清除（cdn::purge_wishlists）', async () => { throw new Blocked('本機沒有 CDN；public API 的 CDN 快取行為無法驗證（R-38 只驗證源站即時結果）') })
await run.step('R-62', '生日前 7 天通知信實際寄出（birthday.rs 排程）', async () => { throw new Blocked('需等待／改動多個使用者生日與排程時間；僅驗證了設定與開關（R-45、R-47）') })
await run.step('R-63', '原生分享面板（navigator.share）、簡訊／WhatsApp／Threads／FB 分享實際開啟', async () => { throw new Blocked('headless Chromium 無 Web Share；外部站台不可達，只驗證 href 內容（R-15）') })

run.save()
await closeBrowser()
