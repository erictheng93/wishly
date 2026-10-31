// 角色流程模擬：公益／慈善組織（承辦人 owner、協調人 coordinator、志工 volunteer、外部捐助者；平台 staff 僅作為推進流程的審核者）
// 設計與結果見 docs/role-flows/03-org.md。執行：cd apps/web && node qa/role-flows/org.mjs
// 原則：能點 UI 就點 UI；只有「平台外」動作（取 OTP / 邀請信、時間快轉改 DB、設 staff、發點）才走 API / SQL，並在 note 註明。
import { mkdirSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { API, Blocked, Gap, MAILPIT, OUT, PASSWORD, WEB, apiCtx, closeBrowser, listMail, newPage, otpFrom, randomUUID, recorder, registerApi, resetOtpLimits, sleep, sql, q, uid, uiLogin, waitForMail } from './lib.mjs'

const run = recorder('org')
const STOP = process.env.STOP_AFTER
const _step = run.step.bind(run)
run.step = async (...a) => { const r = await _step(...a); if (STOP && a[0] === STOP) throw new Error('STOP_AFTER ' + STOP); return r }
const U = uid()
const FILES = resolve(OUT, 'org-files')
mkdirSync(FILES, { recursive: true })

// ---------- 小工具 ----------
const today = () => new Date().toLocaleDateString('sv-SE', { timeZone: 'Asia/Taipei' })
const addDays = (d, n) => { const x = new Date(d + 'T00:00:00Z'); x.setUTCDate(x.getUTCDate() + n); return x.toISOString().slice(0, 10) }
const must = (c, msg) => { if (!c) throw new Error(msg) }
const gap = (c, msg) => { if (!c) throw new Gap(msg) }
const taxId = () => { // 產生通過檢查碼的統編
  for (;;) {
    const s = String(Math.floor(Math.random() * 1e8)).padStart(8, '0'), w = [1, 2, 1, 2, 1, 2, 4, 1]
    const sum = [...s].reduce((a, c, i) => { const p = Number(c) * w[i]; return a + Math.floor(p / 10) + (p % 10) }, 0)
    if (sum % 5 === 0 && s !== '00000000') return s
  }
}
const badTaxId = () => { for (;;) { const s = taxId(); const n = s.slice(0, 7) + String((Number(s[7]) + 1) % 10); if (n !== s) return n } }
const pdfBytes = (pad = 0) => Buffer.concat([Buffer.from('%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n'), Buffer.alloc(pad, 0x20)])
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64')
const f = (name, buf) => { const p = resolve(FILES, name); writeFileSync(p, buf); return p }
const F = {
  reg: f('registration.pdf', pdfBytes(2048)),
  permit: f('permit.pdf', pdfBytes(2048)),
  permitPng: f('permit2.png', PNG),
  txt: f('notes.txt', Buffer.from('not a pdf')),
  big: f('big.pdf', pdfBytes(10.5 * 1024 * 1024)),
  fake: f('fake.pdf', Buffer.from('this is plain text pretending to be a pdf')),
}
const apiOf = async page => apiCtx(await page.context().storageState())
const meOf = async api => (await api.get('me')).json()
const pickOpt = async (sel, re) => { // 以 option 文字挑選（selectOption 的 label 需完全相符）
  const v = await sel.evaluate((el, src) => { const r = new RegExp(src); return [...el.options].find(o => r.test(o.textContent))?.value }, re.source)
  must(v, `找不到選項 ${re}`); await sel.selectOption(v); return v
}
const sheetBtn = (page, name) => page.getByRole('dialog').getByRole('button', { name })

/** 手機版體檢：水平捲動、過小的觸控目標（<44px 高或寬）、被截斷的按鈕文字 */
async function mobileAudit(page) {
  return page.evaluate(() => {
    const vw = innerWidth
    const over = document.documentElement.scrollWidth > vw + 1
    const small = [...document.querySelectorAll('button, a.c-btn, a.g-btn, input:not([type=hidden]), select, textarea')]
      .filter(e => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && (r.height < 40) && getComputedStyle(e).visibility !== 'hidden' })
      .map(e => `${e.tagName.toLowerCase()}${e.className ? '.' + String(e.className).split(' ')[0] : ''}「${(e.textContent || e.getAttribute('aria-label') || e.name || '').trim().slice(0, 12)}」${Math.round(e.getBoundingClientRect().height)}px`)
    const wide = [...document.querySelectorAll('body *')].filter(e => { const r = e.getBoundingClientRect(); return r.right > vw + 2 && r.width > 0 && getComputedStyle(e).position !== 'fixed' }).slice(0, 5).map(e => `${e.tagName.toLowerCase()}.${String(e.className).split(' ')[0]}→right=${Math.round(e.getBoundingClientRect().right)}`)
    return { vw, scrollWidth: document.documentElement.scrollWidth, over, small: small.slice(0, 8), smallCount: small.length, wide }
  })
}

async function uiRegister(page, { name, email, pw = PASSWORD }) {
  resetOtpLimits()
  const t0 = Date.now() - 1000
  await page.getByLabel('顯示名稱').fill(name)
  await page.getByLabel('Email').fill(email)
  await page.locator('input[type=password]').first().fill(pw)
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  const m = await waitForMail(email, { after: t0 })
  await page.locator('.c-otp input').first().fill(otpFrom(m.text))
  const fin = page.getByRole('button', { name: '完成註冊' })
  if (await fin.isVisible().catch(() => false)) await fin.click().catch(() => {})
  await page.waitForURL(u => !/\/register/.test(u.pathname), { timeout: 15000 })
}
/** 後台審核者（staff）打開某組織的詳情卡 */
async function staffOpen(sp, orgId, status = 'pending') {
  await sp.goto('/admin/orgs'); await sp.waitForLoadState('networkidle')
  if (status !== 'pending') { await sp.locator('label.c-field select').first().selectOption(status); await sp.waitForLoadState('networkidle') }
  const card = sp.locator(`article[data-org="${orgId}"]`)
  await card.or(sp.getByText('沒有符合的申請')).first().waitFor({ timeout: 10000 }).catch(() => {})
  for (let i = 0; i < 40 && !(await card.count()); i++) {
    const more = sp.getByRole('button', { name: '載入更多' })
    if (!(await more.count())) break
    await more.click(); await sp.waitForTimeout(600)
  }
  must(await card.count(), `staff 清單（${status}）找不到組織 ${orgId}`)
  await card.getByRole('button', { name: '檢視' }).click()
  await card.locator('dl.c-dl').waitFor({ timeout: 8000 })
  return card
}
const dialogOk = async (sp, name) => { await sp.getByRole('dialog').getByRole('button', { name, exact: true }).click(); await sp.waitForTimeout(700) }

// ---------- 狀態 ----------
const freeDate = () => { // 找一個花蓮縣前後 12 天內沒有任何事件的歷史日期（別的 agent 與先前執行都可能建過事件）
  for (let i = 0; i < 200; i++) {
    const d = addDays(today(), -(30 + Math.floor(Math.random() * 600)))
    if (sql(`SELECT count(*) FROM relief_events WHERE '花蓮縣' = ANY(counties) AND occurred_on BETWEEN '${d}'::date - 12 AND '${d}'::date + 12`) === '0') return d
  }
  throw new Error('找不到沒有事件的日期')
}
const S = { U, evDate: freeDate(), tax: taxId(), shots: {} }
S.orgName = `QA 慈心基金會 ${U}`
S.ownerEmail = `flow-orgowner-${U}@example.com`
S.volEmail = `flow-orgvol-${U}@example.com`
S.ownerName = `陳承辦${U.slice(0, 3)}`
const pages = {}

try {
  // =========================================================== A. 入口與註冊
  pages.guest = await newPage()
  await run.step('O-01', '訪客讀 /org 介紹頁（說明、申請前準備、CTA）', async ({ note }) => {
    const p = pages.guest
    const r = await p.goto('/org')
    must(r.status() === 200, 'status ' + r.status())
    await p.getByRole('heading', { level: 1 }).waitFor()
    const h1 = await p.getByRole('heading', { level: 1 }).innerText()
    const body = await p.locator('main').innerText()
    note(`h1=「${h1}」`)
    for (const s of ['平台審核過的組織', '物資直送收件點', '點數眾籌由平台代購', '可開捐款收據', '立案證書', '統一編號', '勸募許可字號']) must(body.includes(s), '缺 ' + s)
    must(await p.getByRole('link', { name: /申請成為公益組織/ }).isVisible(), '無 CTA')
    note('頁面宣稱「可開捐款收據」；實際捐款收據（donation_receipts）第一段未實作，見問題清單')
  }, { page: pages.guest })

  await run.step('O-02', '登入頁有「公益組織？從這裡申請」入口', async ({ note }) => {
    const p = pages.guest
    await p.goto('/login')
    const l = p.getByRole('link', { name: '公益組織？從這裡申請' })
    await l.waitFor({ timeout: 8000 })
    await l.click(); await p.waitForURL(/\/org$/)
  }, { page: pages.guest })

  await run.step('O-03', '未登入點 CTA → 導向登入且帶 redirect=/org/apply', async ({ note }) => {
    const p = pages.guest
    await p.goto('/org')
    await p.getByRole('link', { name: /申請成為公益組織/ }).click()
    await p.waitForURL(/\/login/)
    note('url=' + p.url())
    must(/redirect=%2Forg%2Fapply|redirect=\/org\/apply/.test(p.url()), 'redirect 參數遺失')
  }, { page: pages.guest })

  pages.owner = await newPage()
  await run.step('O-04', '承辦人由登入頁 →「建立帳號」→ UI 註冊（OTP 由 Mailpit 取）→ 回到 /org/apply', async ({ note }) => {
    const p = pages.owner
    await p.goto('/org'); await p.getByRole('link', { name: /申請成為公益組織/ }).click()
    await p.waitForURL(/\/login/)
    await p.getByRole('link', { name: '建立帳號' }).click()
    await p.waitForURL(/\/register/)
    note('register url=' + p.url())
    await uiRegister(p, { name: S.ownerName, email: S.ownerEmail })
    note('註冊後 url=' + p.url() + '（OTP 取自 Mailpit：平台外動作）')
    await p.waitForURL(/\/org\/apply/, { timeout: 8000 })
  }, { page: pages.owner })
  S.ownerApi = await apiOf(pages.owner)
  S.ownerId = (await meOf(S.ownerApi)).id

  // =========================================================== B. 申請表
  await run.step('O-05', '申請表第 1 步：必填未填時「下一步」鎖住；統編檢查碼錯誤即時提示', async ({ note }) => {
    const p = pages.owner
    await p.goto('/org/apply')
    await p.getByRole('heading', { name: '基本資料' }).waitFor({ timeout: 10000 })
    must(await p.getByRole('button', { name: '下一步' }).isDisabled(), '必填未填時下一步應 disabled')
    await p.getByLabel('統一編號').fill(badTaxId())
    await p.getByLabel('組織全名').click()
    must(await p.getByText('統一編號格式不正確').isVisible(), '統編錯誤無提示')
    await p.getByLabel('統一編號').fill('1234abc')
    note('maxlength=8、inputmode=numeric；輸入 1234abc 後值=' + await p.getByLabel('統一編號').inputValue())
    const sizes = await p.locator('main').screenshot({ path: resolve(OUT, 'org', 'O-05-step1-empty.png') }).then(() => 'ok').catch(() => 'x')
  }, { page: pages.owner })

  await run.step('O-06', '第 1 步填寫（組織類型、統編、立案字號、主管機關、簡介）→ 下一步建立草稿', async ({ note }) => {
    const p = pages.owner
    await p.getByLabel('組織全名').fill(S.orgName)
    await p.getByLabel('對外顯示名稱').fill(S.orgName)
    await p.getByLabel('組織類型').selectOption({ label: '財團法人' })
    await p.getByLabel('統一編號').fill(S.tax)
    await p.getByLabel('立案字號').fill(`內政部台內社字第${U}號`)
    await p.getByLabel('主管機關').fill('內政部')
    await p.getByLabel('官網或粉專').fill('https://example.org/qa')
    await p.getByLabel('組織簡介').fill('QA 測試用：平時做偏鄉兒童營養，災時支援物資集散。')
    must(await p.getByRole('button', { name: '下一步' }).isEnabled(), '下一步未啟用')
    await p.getByRole('button', { name: '下一步' }).click()
    await p.getByRole('heading', { name: '負責人與聯絡' }).waitFor({ timeout: 8000 })
    const me = await meOf(S.ownerApi)
    S.orgId = me.orgs?.[0]?.id
    must(S.orgId, '/me.orgs 無草稿組織')
    note(`草稿 org=${S.orgId} status=${me.orgs[0].verification_status} role=${me.orgs[0].role}`)
  }, { page: pages.owner })

  await run.step('O-07', '第 2 步：負責人與聯絡（欄位 blur 自動保存），上一步/下一步', async ({ note }) => {
    const p = pages.owner
    await p.getByLabel('負責人姓名').fill('陳大文'); await p.getByLabel('負責人姓名').blur()
    await p.getByLabel('聯絡窗口姓名').fill('林小美'); await p.getByLabel('聯絡窗口姓名').blur()
    await p.getByLabel('聯絡窗口職稱').fill('專案經理'); await p.getByLabel('聯絡窗口職稱').blur()
    await p.getByLabel('聯絡窗口手機').fill('0912345678'); await p.getByLabel('聯絡窗口手機').blur()
    await p.getByLabel('公開聯絡 email').fill(`pr-${U}@example.org`); await p.getByLabel('公開聯絡 email').blur()
    await p.getByLabel('公開聯絡電話').fill('02-12345678'); await p.getByLabel('公開聯絡電話').blur()
    await sleep(800)
    const row = sql(`SELECT rep_name||'|'||contact_name||'|'||coalesce(public_email,'') FROM organizations WHERE id='${S.orgId}'`)
    note('DB 已保存：' + row)
    must(row.includes('陳大文') && row.includes('林小美'), '欄位沒被自動保存')
    const ph = sql(`SELECT contact_phone_enc IS NOT NULL AND contact_phone_enc <> '\\x00' FROM organizations WHERE id='${S.orgId}'`)
    note('聯絡手機以密文存放=' + ph)
    await p.getByRole('button', { name: '下一步' }).click()
    await p.getByRole('heading', { name: '證明文件' }).waitFor()
  }, { page: pages.owner })

  await run.step('O-08', '未上傳任何文件就走到第 4 步送審 → 應被擋回第 3 步並說明缺什麼', async ({ note }) => {
    const p = pages.owner
    await p.getByRole('button', { name: '下一步' }).click()
    await p.getByRole('heading', { name: '確認送出' }).waitFor()
    await p.getByLabel('我保證資料真實').check()
    await p.getByRole('button', { name: '送出申請' }).click()
    await p.getByRole('alert').first().waitFor({ timeout: 8000 })
    const msg = await p.getByRole('alert').first().innerText()
    note('錯誤訊息：「' + msg + '」')
    must(await p.getByRole('heading', { name: '證明文件' }).isVisible(), '沒有跳回第 3 步')
    must(/立案|文件|許可|必填|REQUIRED/.test(msg), '訊息沒有說明缺什麼')
  }, { page: pages.owner })

  await run.step('O-09', '上傳檢核：.txt 被擋、11MB 被擋、內容不是 PDF 的 .pdf 檔、正常立案證書', async ({ note }) => {
    const p = pages.owner
    const input = p.locator('input.ap-file').nth(0)
    await input.setInputFiles(F.txt)
    await sleep(1200)
    note('txt 後 alert=' + (await p.getByRole('alert').allInnerTexts()).join('|') + ' problems=' + p.problems.join(';'))
    await p.getByText('僅接受 PDF、JPG、PNG').waitFor({ timeout: 5000 })
    note('txt → 「僅接受 PDF、JPG、PNG」')
    await input.setInputFiles(F.big)
    await p.getByText('檔案超過 10MB').waitFor({ timeout: 5000 })
    note('10.5MB → 「檔案超過 10MB」（前端檢查）')
    await input.setInputFiles(F.fake)
    await sleep(1500)
    const fakeAccepted = await p.getByText(/application\/pdf ·/).count()
    note(`內容是純文字的偽 .pdf：${fakeAccepted ? '被接受（沒有檢查檔案內容）' : '被拒絕'}`)
    if (fakeAccepted) {
      await p.getByRole('button', { name: '刪除' }).first().click(); await sleep(800)
    }
    await input.setInputFiles(F.reg)
    await p.getByText(/application\/pdf · \d+ KB/).first().waitFor({ timeout: 8000 })
    S.fakeAccepted = !!fakeAccepted
    const n = sql(`SELECT count(*) FROM org_documents WHERE org_id='${S.orgId}' AND kind='registration'`)
    note('DB 立案文件數=' + n)
    must(n === '1', '正式文件應為 1 份，實際 ' + n)
    // 檔案確實進了 S3 替身：以 staff 下載在 O-16 驗證
    gap(!fakeAccepted, '偽 PDF（純文字、副檔名 .pdf）被接受：前端只看 file.type（由副檔名推得），後端未驗魔術字節')
  }, { page: pages.owner })

  await run.step('O-10', '新增勸募許可（字號、有效期間起迄、許可函檔案）', async ({ note }) => {
    const p = pages.owner
    must(await p.getByRole('button', { name: '新增許可' }).isDisabled(), '欄位未填時「新增許可」應 disabled')
    S.permitNo = `勸募字第${U}號`
    await p.getByLabel('許可字號').fill(S.permitNo)
    await p.getByLabel('有效期間起').fill(addDays(today(), -30))
    await p.getByLabel('有效期間迄').fill(addDays(today(), 180))
    await p.locator('input.ap-file').nth(1).setInputFiles(F.permit)
    await p.getByRole('button', { name: '新增許可' }).click()
    await p.getByText(S.permitNo).waitFor({ timeout: 8000 })
    note('清單出現 ' + S.permitNo)
    S.p1 = sql(`SELECT id FROM org_permits WHERE org_id='${S.orgId}' AND permit_no='${S.permitNo}'`)
    note('許可狀態=' + sql(`SELECT status FROM org_permits WHERE id='${S.p1}'`))
    // 邊界：結束日早於開始日
    await p.getByLabel('許可字號').fill('反向日期')
    await p.getByLabel('有效期間起').fill(addDays(today(), 10))
    await p.getByLabel('有效期間迄').fill(addDays(today(), 1))
    await p.locator('input.ap-file').nth(1).setInputFiles(F.permitPng)
    await p.getByRole('button', { name: '新增許可' }).click()
    await sleep(1800)
    const bad = sql(`SELECT count(*) FROM org_permits WHERE org_id='${S.orgId}' AND permit_no='反向日期'`)
    const al = (await p.getByRole('alert').count()) ? await p.getByRole('alert').first().innerText() : ''
    const onStep3 = await p.getByRole('heading', { name: '證明文件' }).isVisible()
    const orphan = sql(`SELECT count(*) FROM org_documents WHERE org_id='${S.orgId}' AND kind='permit'`)
    note(`迄日早於起日：DB 筆數=${bad}；畫面訊息=「${al}」；仍停在第 3 步=${onStep3}；許可文件數=${orphan}（失敗時前端會刪除孤兒文件）`)
    must(bad === '0', '起訖顛倒的許可被存進去了')
    S.permitJumped = !onStep3
    // 回到第 3 步繼續
    for (let i = 0; i < 3 && !(await p.getByRole('heading', { name: '證明文件' }).isVisible()); i++) await p.getByRole('button', { name: '下一步' }).click()
    gap(onStep3, '許可日期驗證失敗（pointer=/valid_to）時，申請表被導回第 1 步（apply.vue:34 ptrStep 對未知 pointer 預設 1），使用者在第 3 步按「新增許可」卻被丟到「基本資料」，錯誤訊息還留在頁面頂端')
  }, { page: pages.owner })

  await run.step('O-11', '重新整理後草稿從正確步驟還原；每人最多 1 份草稿（再 POST /orgs → 409）', async ({ note }) => {
    const p = pages.owner
    await p.reload(); await p.waitForLoadState('networkidle')
    await p.getByRole('heading', { name: '確認送出' }).waitFor({ timeout: 10000 })
    note('重整後直接回到第 4 步（stepOf 判斷資料齊全）')
    const r = await S.ownerApi.post('orgs', { data: { legal_name: '重複草稿', display_name: '重複草稿' } })
    note(`再 POST /orgs → ${r.status()} ${(await r.json().catch(() => ({}))).code ?? ''}`)
    must(r.status() === 409, '應為 409，實際 ' + r.status())
  }, { page: pages.owner })

  await run.step('O-12', '第 4 步預覽資料 → 未勾同意時送出鎖住 → 勾選送出 → 進入「審核中」', async ({ note }) => {
    const p = pages.owner
    const dl = await p.locator('.ap-s4 dl').innerText()
    must(dl.includes(S.orgName) && dl.includes(S.tax), '預覽缺資料')
    must(await p.getByRole('button', { name: '送出申請' }).isDisabled(), '未勾同意時應 disabled')
    await p.getByLabel('我保證資料真實').check()
    await p.getByRole('button', { name: '送出申請' }).click()
    await p.getByText('審核中：預計 3 個工作天內完成').waitFor({ timeout: 8000 })
    note('狀態=' + sql(`SELECT verification_status FROM organizations WHERE id='${S.orgId}'`))
  }, { page: pages.owner })

  await run.step('O-13', '審核中：表單唯讀、可撤回（回草稿可編輯）、再送審', async ({ note }) => {
    const p = pages.owner
    must(await p.getByRole('button', { name: '撤回申請' }).isVisible(), '無撤回鈕')
    const roApi = await S.ownerApi.patch(`orgs/${S.orgId}`, { data: { intro: '審核中偷改' } })
    note(`審核中 PATCH 簡介 → ${roApi.status()} ${(await roApi.json().catch(() => ({}))).code ?? ''}`)
    must(roApi.status() === 409, '審核中應不可改')
    await p.getByRole('button', { name: '撤回申請' }).click()
    await p.getByRole('heading', { name: /基本資料|負責人|證明文件|確認送出/ }).first().waitFor({ timeout: 8000 })
    note('撤回後 DB 狀態=' + sql(`SELECT verification_status FROM organizations WHERE id='${S.orgId}'`))
    must(sql(`SELECT verification_status FROM organizations WHERE id='${S.orgId}'`) === 'draft', '未回 draft')
    await p.getByRole('button', { name: '下一步' }).count()
    // 撤回後編輯簡介再送審
    await p.reload(); await p.waitForLoadState('networkidle')
    await p.getByRole('heading', { name: '確認送出' }).waitFor({ timeout: 8000 })
    await p.getByRole('button', { name: '上一步' }).click(); await p.getByRole('button', { name: '上一步' }).click(); await p.getByRole('button', { name: '上一步' }).click()
    await p.getByLabel('組織簡介').fill('QA 測試用（撤回後修改）：平時做偏鄉兒童營養，災時支援物資集散。'); await p.getByLabel('組織簡介').blur(); await sleep(700)
    await p.getByRole('button', { name: '下一步' }).click(); await p.getByRole('button', { name: '下一步' }).click(); await p.getByRole('button', { name: '下一步' }).click()
    await p.getByLabel('我保證資料真實').check()
    await p.getByRole('button', { name: '送出申請' }).click()
    await p.getByText('審核中：預計 3 個工作天內完成').waitFor({ timeout: 8000 })
  }, { page: pages.owner })

  await run.step('O-14', '審核中（未通過）嘗試建立事件／據點 → 被擋並說明', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/orgs/${S.orgId}/events/new`)
    await p.locator('select[name=county]').selectOption('花蓮縣')
    await p.locator('input[name=occurred_on]').fill(S.evDate)
    await p.locator('input[name=name]').waitFor({ timeout: 8000 })
    await p.locator('input[name=name]').fill('未審先建')
    await p.getByRole('button', { name: '建立事件' }).click()
    const a = await p.getByRole('alert').first().innerText({ timeout: 8000 })
    note('事件：「' + a + '」')
    must(a.includes('尚未通過審核'), '訊息不符')
    const r = await S.ownerApi.post(`orgs/${S.orgId}/sites`, { data: { title: 'x', event_id: randomUUID(), permit_id: S.p1, site_address: '花蓮縣某處一號', recipient_name: 'a', recipient_phone: '0912345678' } })
    note(`據點 API（審核中）→ ${r.status()} ${(await r.json().catch(() => ({}))).code ?? ''}`)
    // 規格 5.1 說審核期間可先建草稿清單；實作：建事件需 verified，據點需事件 → 實際上無法預先準備
    gap(false, '規格 §5.1 寫「審核期間可以先建立草稿清單」，但建事件需 verified、建據點需事件與已核准許可，審核中實際無法預備草稿（設計與實作不一致）')
  }, { page: pages.owner })

  pages.b = await registerApi({ name: `申請人B${U.slice(0, 3)}`, tag: 'orgb' })
  S.bPage = await newPage({ user: pages.b })
  await run.step('O-15', '另一位申請人 B 用相同統編：第 1 步不擋、送審才被擋（提示聯絡平台）→ 換統編後送審', async ({ note }) => {
    const p = S.bPage
    await p.goto('/org/apply'); await p.getByRole('heading', { name: '基本資料' }).waitFor({ timeout: 10000 })
    await p.getByLabel('組織全名').fill(`QA 撞統編協會 ${U}`)
    await p.getByLabel('對外顯示名稱').fill(`撞統編協會 ${U}`)
    await p.getByLabel('組織類型').selectOption({ label: '社團法人' })
    await p.getByLabel('統一編號').fill(S.tax)
    await p.getByLabel('立案字號').fill('台內社字第001號')
    await p.getByLabel('主管機關').fill('內政部')
    await p.getByRole('button', { name: '下一步' }).click()
    await p.getByRole('heading', { name: '負責人與聯絡' }).waitFor({ timeout: 8000 })
    note('用與審核中組織相同的統編建立草稿：第 1 步「下一步」成功（統編唯一性只在送審時檢查，partial unique index 不含 draft）')
    // B 補齊文件、許可、聯絡人（API：上傳 UI 已於 O-09/O-10 驗證，這裡省時間）
    const api = pages.b.api
    const bid = (await meOf(api)).orgs[0].id; S.bId = bid
    for (const [k, v] of [['rep_name', '王B'], ['contact_name', '李B'], ['contact_phone', '0922333444'], ['public_email', `b-${U}@example.org`]]) await api.patch(`orgs/${bid}`, { data: { [k]: v } })
    must((await api.post(`orgs/${bid}/documents?kind=registration`, { data: pdfBytes(1024), headers: { 'Content-Type': 'application/pdf' } })).ok(), 'B 上傳立案')
    const d = await (await api.post(`orgs/${bid}/documents?kind=permit`, { data: pdfBytes(1024), headers: { 'Content-Type': 'application/pdf' } })).json()
    must((await api.post(`orgs/${bid}/permits`, { data: { permit_no: `B-${U}`, valid_from: addDays(today(), -5), valid_to: addDays(today(), 100), document_id: d.id } })).ok(), 'B 許可')
    note('（API）B 補齊聯絡人／文件／許可：平台外替身動作')
    await p.reload(); await p.getByRole('heading', { name: '確認送出' }).waitFor({ timeout: 10000 })
    await p.getByLabel('我保證資料真實').check(); await p.getByRole('button', { name: '送出申請' }).click()
    await p.locator('small.c-err').first().waitFor({ timeout: 8000 })
    const m = await p.locator('small.c-err').first().innerText()
    const top = await p.getByRole('alert').first().innerText().catch(() => '')
    note(`送審撞統編：欄位提示=「${m}」；頁首訊息=「${top}」；被導回第 1 步=${await p.getByRole('heading', { name: '基本資料' }).isVisible()}`)
    must(await p.getByRole('heading', { name: '基本資料' }).isVisible(), '應回到第 1 步')
    S.bTax = taxId()
    await p.getByLabel('統一編號').fill(S.bTax); await p.getByLabel('統一編號').blur(); await sleep(800)
    for (let i = 0; i < 3; i++) await p.getByRole('button', { name: '下一步' }).click()
    await p.getByLabel('我保證資料真實').check(); await p.getByRole('button', { name: '送出申請' }).click()
    await p.getByText('審核中：預計 3 個工作天內完成').waitFor({ timeout: 8000 })
    note('換統編後 B 送審成功（pending）')
    gap(false, '統編重複要等到第 4 步送出才被偵測（第 1 步即時只檢查格式與檢查碼）；B 已填完四步、上傳文件才知道撞統編。建議第 1 步 blur 時呼叫檢查端點，或至少在草稿建立時提示')
  }, { page: S.bPage })

  // =========================================================== C. 審核往返（staff 為推進流程所需）
  pages.staffUser = await registerApi({ staff: true, tag: 'orgstaff' })
  pages.staff = await newPage({ user: pages.staffUser })
  await run.step('O-16', '【staff】在 /admin/orgs 看到待審申請、檢視資料並下載立案文件', async ({ note }) => {
    const sp = pages.staff
    const card = await staffOpen(sp, S.orgId)
    const dl = await card.locator('dl.c-dl').innerText()
    must(dl.includes(S.tax), '詳情缺統編')
    const [download] = await Promise.all([sp.waitForEvent('download', { timeout: 10000 }), card.getByRole('button', { name: '下載' }).first().click()])
    const path = resolve(FILES, 'staff-downloaded-' + download.suggestedFilename())
    await download.saveAs(path)
    note('下載檔名=' + download.suggestedFilename() + '（文件由私有儲存經短效網址取回）')
    note('手機欄位在 staff 畫面：' + (dl.includes('0912345678') ? '顯示明文（審核用）' : '未顯示'))
  }, { page: pages.staff })

  await run.step('O-17', '【staff】許可尚未核准就核准組織 → 被擋', async ({ note }) => {
    const sp = pages.staff
    const card = sp.locator(`article[data-org="${S.orgId}"]`)
    await card.getByRole('button', { name: '核准組織', exact: true }).click()
    await sp.getByRole('dialog').getByRole('button', { name: '核准組織', exact: true }).click()
    const a = await sp.getByRole('dialog').getByRole('alert').innerText({ timeout: 8000 })
    note('錯誤：「' + a + '」')
    await sp.getByRole('dialog').getByRole('button', { name: '取消' }).click()
  }, { page: pages.staff })

  await run.step('O-18', '【staff】要求補件：勾選 legal_name 與 documents、寫意見', async ({ note }) => {
    const sp = pages.staff
    const card = sp.locator(`article[data-org="${S.orgId}"]`)
    await card.getByRole('button', { name: '要求補件' }).click()
    await sp.getByRole('dialog').getByRole('button', { name: '要求補件', exact: true }).click() // 未勾欄位
    note('未勾欄位送出：' + await sp.getByRole('dialog').getByRole('alert').innerText({ timeout: 5000 }))
    await sp.locator('input[name="flag-legal_name"]').check()
    await sp.locator('input[name="flag-registration_no"]').check()
    await sp.locator('input[name="flag-documents"]').check()
    await sp.getByRole('dialog').getByLabel('審核備註').fill('立案證書影本不清楚，請重新上傳；全名請與證書一致，並補正立案字號。')
    await dialogOk(sp, '要求補件')
    await sp.getByText('已處理').first().waitFor({ timeout: 8000 })
    note('DB 狀態=' + sql(`SELECT verification_status||' flagged='||flagged_fields::text FROM organizations WHERE id='${S.orgId}'`))
  }, { page: pages.staff })

  await run.step('O-19', '承辦人看到「需補件」與意見；只有被點名欄位可改，其餘鎖定；是否收到通知信', async ({ note }) => {
    const p = pages.owner
    await p.goto('/org/apply'); await p.waitForLoadState('networkidle')
    await p.getByText('需補件：請修改被點名的欄位後重新送出').waitFor({ timeout: 10000 })
    const txt = await p.locator('.c-card[role=status]').first().innerText()
    note('畫面：' + txt.replace(/\n/g, ' / '))
    must(txt.includes('立案證書影本不清楚'), '看不到審核意見')
    const dis = {}
    for (const l of ['組織全名', '對外顯示名稱', '立案字號', '主管機關', '組織簡介']) dis[l] = await p.getByLabel(l).first().isDisabled()
    note('欄位 disabled：' + JSON.stringify(dis))
    must(!dis['組織全名'] && !dis['立案字號'] && dis['對外顯示名稱'] && dis['主管機關'], '鎖定邏輯不符')
    const nc = await waitForMail(S.ownerEmail, { subject: /補件/, timeout: 30000 }).catch(() => null)
    note('補件通知信：' + (nc ? `「${nc.subject}」 內文=${(nc.text || '').replace(/\s+/g, ' ').slice(0, 260)}` : '30 秒內沒收到'))
    if (nc) { note('信中含審核意見=' + nc.text.includes('立案證書影本不清楚') + '；含申請頁連結=' + /\/org\/apply/.test(nc.text + nc.html) + '；含被點名欄位=' + /立案字號|文件|全名/.test(nc.text)); S.ncMail = nc.text }
    // 文件被點名：能否換文件？
    await p.getByRole('button', { name: '下一步' }).click().catch(() => {})
  }, { page: pages.owner })

  await run.step('O-20', '補件：改全名與立案字號、換立案證書（含刪除舊檔）→ 重新送審', async ({ note }) => {
    const p = pages.owner
    await p.reload(); await p.waitForLoadState('networkidle')
    await p.getByLabel('組織全名').fill(S.orgName + '（補正）'); await p.getByLabel('組織全名').blur()
    await p.getByLabel('立案字號').fill(`內政部台內社字第${U}號（補正）`); await p.getByLabel('立案字號').blur()
    await sleep(800)
    await p.getByRole('button', { name: '下一步' }).click()
    await p.getByRole('heading', { name: '負責人與聯絡' }).waitFor()
    await p.getByRole('button', { name: '下一步' }).click()
    await p.getByRole('heading', { name: '證明文件' }).waitFor()
    const del = p.getByRole('button', { name: '刪除' })
    const delDisabled = await del.first().isDisabled().catch(() => null)
    note('補件時刪除舊文件鈕 disabled=' + delDisabled)
    const before = sql(`SELECT count(*) FROM org_documents WHERE org_id='${S.orgId}' AND kind='registration'`)
    await p.getByRole('listitem').filter({ hasText: 'KB' }).first().getByRole('button', { name: '刪除' }).click()
    await sleep(1200)
    note(`刪除舊立案文件：${before} → ` + sql(`SELECT count(*) FROM org_documents WHERE org_id='${S.orgId}' AND kind='registration'`))
    await p.locator('input.ap-file').nth(0).setInputFiles(F.permitPng)
    await sleep(1500)
    note('立案文件數=' + sql(`SELECT count(*) FROM org_documents WHERE org_id='${S.orgId}' AND kind='registration'`))
    const docsAlert = (await p.getByRole('alert').count()) ? await p.getByRole('alert').first().innerText() : ''
    if (docsAlert) note('上傳訊息：' + docsAlert)
    await p.getByRole('button', { name: '下一步' }).click()
    await p.getByLabel('我保證資料真實').check()
    await p.getByRole('button', { name: '送出申請' }).click()
    await p.getByText('審核中：預計 3 個工作天內完成').waitFor({ timeout: 8000 })
    note('重送後 flagged_fields=' + sql(`SELECT flagged_fields::text FROM organizations WHERE id='${S.orgId}'`) + '；review_note=' + sql(`SELECT coalesce(review_note,'NULL') FROM organizations WHERE id='${S.orgId}'`))
  }, { page: pages.owner })

  await run.step('O-21', '【staff】核准勸募許可 → 核准組織', async ({ note }) => {
    const sp = pages.staff
    const card = await staffOpen(sp, S.orgId)
    const permit = card.locator(`[data-permit="${S.p1}"]`)
    await permit.getByRole('button', { name: '核准許可' }).click(); await dialogOk(sp, '核准許可')
    await permit.getByText('已核准').first().waitFor({ timeout: 8000 })
    await card.getByRole('button', { name: '核准組織', exact: true }).click(); await dialogOk(sp, '核准組織')
    await sp.getByText('已處理').first().waitFor({ timeout: 8000 })
    note('DB 狀態=' + sql(`SELECT verification_status FROM organizations WHERE id='${S.orgId}'`))
  }, { page: pages.staff })

  await run.step('O-22', '承辦人狀態頁顯示「已通過審核」→ 按「切換到組織身分」進 /orgs/[id]；核准通知信', async ({ note }) => {
    const p = pages.owner
    await p.goto('/org/apply'); await p.waitForLoadState('networkidle')
    await p.getByText('已通過審核，可以切換到組織身分了').waitFor({ timeout: 10000 })
    await p.getByRole('button', { name: '切換到組織身分' }).click()
    await p.waitForURL(new RegExp(`/orgs/${S.orgId}$`), { timeout: 8000 })
    const ap = await waitForMail(S.ownerEmail, { subject: /已通過審核/, timeout: 30000 }).catch(() => null)
    note('核准通知信：' + (ap ? `「${ap.subject}」 內文=${(ap.text || '').replace(/\s+/g, ' ').slice(0, 220)}；含連結=${/https?:\/\//.test(ap.text)}` : '30 秒內沒收到'))
  }, { page: pages.owner })

  await run.step('O-23', '【申請人 B】staff 退件 → B 看到未通過、原因與 30 天倒數 → 快轉後重新申請', async ({ note }) => {
    const bp = S.bPage, bid = S.bId
    const sp = pages.staff
    const card = await staffOpen(sp, bid)
    await card.getByRole('button', { name: '退件', exact: true }).last().click()
    await sp.getByRole('dialog').getByLabel('審核備註').fill('立案字號查無資料，退件。')
    await dialogOk(sp, '退件')
    await sp.getByText('已處理').first().waitFor({ timeout: 8000 })
    await bp.reload(); await bp.waitForLoadState('networkidle')
    await bp.getByText('未通過審核').waitFor({ timeout: 8000 })
    const t = await bp.locator('.ap-state').innerText()
    note('B 畫面：' + t.replace(/\n/g, ' / '))
    must(t.includes('立案字號查無資料') && /之後（30 天後）可重新申請/.test(t), '缺少原因或倒數')
    const early = await pages.b.api.patch(`orgs/${bid}`, { data: { legal_name: '提早重申' } })
    note(`30 天內直接 PATCH 重新申請（API）→ ${early.status()} ${(await early.json().catch(() => ({}))).code ?? ''}`)
    sql(`UPDATE organizations SET resubmit_after = now() - interval '1 minute' WHERE id='${bid}'`)
    note('（SQL）把 resubmit_after 快轉到過去：時間快轉屬平台外動作')
    await bp.reload(); await bp.waitForLoadState('networkidle')
    await bp.getByRole('button', { name: '重新申請' }).click()
    await bp.getByRole('heading', { name: /基本資料|負責人|證明文件|確認送出/ }).first().waitFor({ timeout: 8000 })
    note('重新申請後狀態=' + sql(`SELECT verification_status FROM organizations WHERE id='${bid}'`) + '；review_note=' + sql(`SELECT coalesce(review_note,'NULL') FROM organizations WHERE id='${bid}'`) + '；舊文件數=' + sql(`SELECT count(*) FROM org_documents WHERE org_id='${bid}'`))
  }, { page: S.bPage })

  // =========================================================== D. 組織頁
  await run.step('O-24', '組織後台 /orgs/[id]：組織模式標記、身分選單、空狀態、owner 才有「成員與邀請」', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/orgs/${S.orgId}`); await p.waitForLoadState('networkidle')
    await p.getByRole('heading', { level: 1 }).first().waitFor()
    note('h1=' + await p.getByRole('heading', { level: 1 }).first().innerText())
    must(await p.getByText(/組織模式：/).isVisible(), '無「組織模式」標記')
    must(await p.getByText('尚無事件').isVisible() && await p.getByText('尚無據點').isVisible(), '空狀態文案缺')
    must(await p.getByRole('link', { name: '成員與邀請' }).isVisible(), 'owner 看不到成員入口')
    await p.getByRole('button', { name: new RegExp(S.ownerName) }).click()
    const items = await p.locator('.um-pop button').allInnerTexts()
    note('身分選單：' + JSON.stringify(items))
    S.orgNameShown = items.find(i => i.includes('QA 慈心'))
    await p.keyboard.press('Escape')
    const slug = (await (await S.ownerApi.get(`orgs/${S.orgId}`)).json()).slug
    S.orgSlug = slug; note('slug=' + slug)
  }, { page: pages.owner })

  await run.step('O-25', '公開組織頁 /o/[slug]（尚無據點）：已驗證徽章、聯絡資訊、SSR 狀態', async ({ note }) => {
    const p = pages.guest
    const r = await p.goto(`/o/${S.orgSlug}`)
    must(r.status() === 200, 'status ' + r.status())
    const t = await p.locator('main').innerText()
    note(t.replace(/\n+/g, ' / ').slice(0, 220))
    must(t.includes('已驗證組織') && t.includes(`pr-${U}@example.org`) && t.includes('目前沒有收件中的據點'), '內容不符')
    must(!t.includes('0912345678'), '洩漏審核用手機！')
    const og = await p.locator('meta[property="og:title"]').getAttribute('content')
    note('og:title=' + og)
    const bad = await p.goto('/o/nonexistent-' + U)
    note('不存在 slug HTTP ' + bad.status())
  }, { page: pages.guest })

  await run.step('O-26', 'verified 後：編輯組織資料／新增第二張許可有無 UI 入口；API 行為', async ({ note }) => {
    const p = pages.owner
    await p.goto('/org/apply'); await p.waitForLoadState('networkidle')
    const txt = await p.locator('main').innerText()
    const hasForm = await p.locator('input').count()
    note(`已通過狀態頁 input 數=${hasForm}（無許可/資料編輯入口）`)
    const r = await S.ownerApi.patch(`orgs/${S.orgId}`, { data: { intro: '通過後改簡介' } })
    note(`verified 後 PATCH 簡介 → ${r.status()} ${(await r.json().catch(() => ({}))).code ?? ''}`)
    // 新增第二張許可（為後續到期／撤銷測試）— UI 無入口，只能走 API
    const d = await (await S.ownerApi.post(`orgs/${S.orgId}/documents?kind=permit`, { data: pdfBytes(1024), headers: { 'Content-Type': 'application/pdf' } })).json()
    S.permitNo2 = `勸募二號${U}`
    const pr = await S.ownerApi.post(`orgs/${S.orgId}/permits`, { data: { permit_no: S.permitNo2, valid_from: addDays(today(), -10), valid_to: addDays(today(), 60), document_id: d.id } })
    note(`新增第二張許可（API）→ ${pr.status()}`)
    S.p2 = (await pr.json()).id
    // staff 以 UI 核准
    const sp = pages.staff
    const card = await staffOpen(sp, S.orgId, 'verified')
    const permit = card.locator(`[data-permit="${S.p2}"]`)
    await permit.getByRole('button', { name: '核准許可' }).click(); await dialogOk(sp, '核准許可')
    await permit.getByText('已核准').first().waitFor({ timeout: 8000 })
    gap(false, '組織通過後無任何 UI 可新增許可或修改簡介/聯絡資訊/Logo（/orgs/[id] 只有事件、據點、成員）；據點表單的 stNoPermit 文案指示「到組織資料補件」，但該頁不存在')
  }, { page: pages.owner })

  // =========================================================== E. 成員
  pages.coord = await registerApi({ name: `協調員林${U.slice(0, 3)}`, tag: 'orgcoord' })
  pages.vol = null
  await run.step('O-27', '成員頁初始狀態：只有自己（擁有者）；邀請表單欄位', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/orgs/${S.orgId}/members`); await p.waitForLoadState('networkidle')
    await p.getByRole('heading', { name: '成員' }).first().waitFor()
    const li = await p.locator('.m-list li').allInnerTexts()
    note('成員：' + JSON.stringify(li))
    must(li.length === 1 && li[0].includes('擁有者'), '初始成員不符')
    must(await p.getByText('沒有待接受的邀請').isVisible(), '待接受邀請空狀態缺')
    const roleOpts = await p.locator('form.m-form select option').allInnerTexts()
    note('邀請角色選項：' + roleOpts.join('/') + '；預設=' + await p.locator('form.m-form select').inputValue())
    must(await p.locator('form.m-form button').isDisabled(), '未填 email 時「寄出邀請」應 disabled')
  }, { page: pages.owner })

  await run.step('O-28', '邀請 coordinator（對方已有帳號）：產生連結／QR、列入待接受、收到邀請信', async ({ note }) => {
    const p = pages.owner
    const t0 = Date.now() - 1000
    await p.getByLabel('Email').fill(pages.coord.email)
    await p.locator('form.m-form select').selectOption('coordinator')
    await p.getByLabel('顯示代號').fill('coord1')
    await p.getByRole('button', { name: '寄出邀請' }).click()
    S.coordLink = await p.getByLabel('邀請連結').inputValue()
    note('連結：' + S.coordLink.replace(/[0-9A-Za-z_-]{20,}/, '<token>'))
    must(await p.locator('.m-link svg, .m-link canvas, .m-link img').count() > 0, 'QR 沒出現')
    await p.locator('.m-list li', { hasText: pages.coord.email }).waitFor({ timeout: 8000 })
    const m = await waitForMail(pages.coord.email, { after: t0, timeout: 25000 })
    note(`信件主旨：「${m.subject}」；內含連結=${(m.text + m.html).includes('/org/invite/')}`)
    S.coordMail = m
    must((m.text + m.html).includes('/org/invite/'), '信中沒有邀請連結')
  }, { page: pages.owner })

  pages.coordPage = await newPage() // 全新瀏覽器，沒有任何登入
  await run.step('O-29', '協調員從信件連結開啟 /org/invite/[token]：未登入→導登入→接受→進組織；看不到「成員與邀請」', async ({ note }) => {
    const p = pages.coordPage
    const link = (S.coordMail.text.match(/https?:\/\/\S*\/org\/invite\/[\w-]+/) ?? [S.coordLink])[0]
    await p.goto(new URL(link).pathname)
    await p.waitForURL(/\/login/, { timeout: 10000 })
    note('未登入 → ' + p.url())
    await p.getByLabel(/email|電子郵件/i).first().fill(pages.coord.email)
    await p.getByLabel(/密碼/).first().fill(PASSWORD)
    await p.getByRole('button', { name: /^登入$/ }).click()
    await p.waitForURL(u => u.pathname.startsWith('/org/invite/'), { timeout: 15000 })
    await p.getByRole('button', { name: '接受邀請' }).waitFor({ timeout: 8000 })
    const text = await p.locator('main').innerText()
    note('邀請頁：' + text.replace(/\n/g, ' / '))
    must(text.includes('邀請你擔任「協調員」'), '邀請文案不符')
    await p.getByRole('button', { name: '接受邀請' }).click()
    await p.waitForURL(new RegExp(`/orgs/${S.orgId}$`), { timeout: 10000 })
    await p.waitForLoadState('networkidle')
    must(await p.getByText(/組織模式：/).isVisible(), '無組織模式標記')
    must(await p.getByRole('link', { name: '成員與邀請' }).count() === 0, 'coordinator 不應看到成員管理入口')
    await p.getByRole('link', { name: '新增事件' }).waitFor({ timeout: 8000 }).catch(() => {})
    must(await p.getByRole('link', { name: '新增事件' }).isVisible() && await p.getByRole('link', { name: '新增據點' }).isVisible(), 'coordinator 應可新增事件/據點')
    const again = await pages.coord.api.post(`org-invites/${S.coordLink.split('/org/invite/')[1]}/accept`)
    note(`同一連結再接受一次 → ${again.status()} ${(await again.json().catch(() => ({}))).code ?? ''}`)
  }, { page: pages.coordPage })

  // 志工：先邀請「尚未註冊」的 email
  pages.volPage = await newPage()
  await run.step('O-30', '邀請尚未註冊的志工 email：有無寄信？轉交連結後志工註冊→接受', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/orgs/${S.orgId}/members`); await p.waitForLoadState('networkidle')
    const t0 = Date.now() - 1000
    await p.getByLabel('Email').fill(S.volEmail)
    await p.locator('form.m-form select').selectOption('volunteer')
    await p.getByRole('button', { name: '寄出邀請' }).click()
    S.volLink = await p.getByLabel('邀請連結').inputValue()
    await sleep(4000)
    const mails = await listMail(S.volEmail)
    note(`邀請寄給尚未註冊的 ${S.volEmail}：收到信件數=${mails.length}`)
    note('畫面提示：' + await p.locator('.m-link p').innerText())
    // 志工用該 email 註冊（UI），再開連結接受
    const vp = pages.volPage
    await vp.goto(new URL(S.volLink).pathname)
    await vp.waitForURL(/\/login/)
    await vp.getByRole('link', { name: '建立帳號' }).click()
    await vp.waitForURL(/\/register/)
    await uiRegister(vp, { name: `志工小王${U.slice(0, 3)}`, email: S.volEmail })
    note('註冊後 url=' + vp.url())
    if (!/\/org\/invite\//.test(vp.url())) await vp.goto(new URL(S.volLink).pathname)
    await vp.getByRole('button', { name: '接受邀請' }).click()
    await vp.waitForURL(new RegExp(`/orgs/${S.orgId}$`), { timeout: 10000 })
    S.volApi = await apiOf(vp); S.volId = (await meOf(S.volApi)).id
    gap(mails.length > 0, '對尚未註冊的 email 不寄邀請信（org_members.rs:177 只通知既有使用者），owner 必須自行複製連結轉交；規格 §5.2「透過 notify outbox 寄信」與申請頁「對方還沒有帳號時可以先註冊再加入」的情境無信可收')
  }, { page: pages.volPage })

  pages.out = await registerApi({ name: `路人甲${U.slice(0, 3)}`, tag: 'orgout' })
  pages.outPage = await newPage({ user: pages.out })
  await run.step('O-31', '邀請 email 不符：另一個帳號開連結 → 提示用哪個 email 登入', async ({ note }) => {
    const l = await (await S.ownerApi.post(`orgs/${S.orgId}/invites`, { data: { email: `someone-${U}@example.com`, role: 'volunteer' } })).json()
    note('（API）建立另一封邀請以測試 email 不符')
    const op = pages.outPage
    await op.goto(new URL(l.url).pathname)
    await op.getByRole('button', { name: '接受邀請' }).click()
    const a = await op.getByRole('alert').innerText({ timeout: 8000 })
    note('錯誤：「' + a + '」')
    must(a.includes(`someone-${U}@example.com`), '提示沒帶入受邀 email')
    S.mismatchInviteId = l.id
  }, { page: pages.outPage })

  await run.step('O-32', '取消邀請：待接受清單消失；已取消連結再開 → 錯誤', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/orgs/${S.orgId}/members`); await p.waitForLoadState('networkidle')
    const row = p.locator('.m-list li', { hasText: `someone-${U}@example.com` })
    await row.getByRole('button', { name: '取消邀請' }).click()
    await row.waitFor({ state: 'detached', timeout: 8000 })
    const r = await pages.outPage.request.get(`${API}/api/v1/org-invites/x`).catch(() => null)
    const ph = await (await S.ownerApi.get(`orgs/${S.orgId}/invites`)).json()
    note('取消後 invites 紀錄：' + JSON.stringify(ph.map(i => ({ e: i.email.slice(0, 8), c: !!i.cancelled_at, a: !!i.accepted_at }))))
    const op = pages.outPage
    // 取消的連結：重新取得 token 不可能（只存 hash）；用先前連結
    const inv = await (await S.ownerApi.post(`orgs/${S.orgId}/invites`, { data: { email: pages.out.email, role: 'volunteer' } })).json()
    await S.ownerApi.delete(`orgs/${S.orgId}/invites/${inv.id}`)
    await op.goto(new URL(inv.url).pathname)
    await op.waitForLoadState('networkidle')
    const msg = (await op.getByRole('alert').count()) ? await op.getByRole('alert').first().innerText() : await op.locator('main').innerText()
    note('開已取消邀請：「' + msg.replace(/\n/g, ' ') + '」')
    must(!(await op.getByRole('button', { name: '接受邀請' }).count()), '已取消邀請仍可接受')
  }, { page: pages.owner })

  await run.step('O-33', '角色變更：志工→協調員→志工（owner 於成員列表下拉）；被改者權限即時改變', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/orgs/${S.orgId}/members`); await p.waitForLoadState('networkidle')
    const row = p.locator(`[data-testid="member-${S.volId}"]`)
    await row.getByLabel('角色').selectOption('coordinator')
    await sleep(1200)
    note('升級後 DB 角色=' + sql(`SELECT role FROM org_members WHERE org_id='${S.orgId}' AND user_id='${S.volId}'`))
    const r = await S.volApi.get(`orgs/${S.orgId}/members`)
    const vp = pages.volPage
    await vp.goto(`/orgs/${S.orgId}`); await vp.waitForLoadState('networkidle')
    must(await vp.getByRole('link', { name: '新增事件' }).isVisible(), '升為協調員後看不到新增事件（/me 可能快取）')
    await row.getByLabel('角色').selectOption('volunteer')
    await sleep(1200)
    must(sql(`SELECT role FROM org_members WHERE org_id='${S.orgId}' AND user_id='${S.volId}'`) === 'volunteer', '降回志工失敗')
    const aud = sql(`SELECT count(*) FROM audit_logs WHERE action='org.member.update' AND (diff->>'org_id')='${S.orgId}'`).trim()
    note('audit_logs org.member.update 筆數=' + aud)
  }, { page: pages.owner })

  await run.step('O-34', '停用成員（確認視窗）→ 對方立即失去存取；重新啟用', async ({ note }) => {
    const p = pages.owner
    const row = p.locator(`[data-testid="member-${S.volId}"]`)
    await row.getByRole('button', { name: '停用' }).click()
    await p.getByRole('dialog').getByText(/確定要停用/).waitFor()
    await sheetBtn(p, '確認').click(); await sleep(1200)
    must((await row.innerText()).includes('已停用'), '列表未標示已停用')
    const vp = pages.volPage
    const r = await S.volApi.get(`orgs/${S.orgId}/sites`)
    note(`被停用者 GET sites → ${r.status()}`)
    must([403, 404].includes(r.status()), '被停用者仍可存取 ' + r.status())
    await vp.goto(`/orgs/${S.orgId}`); await vp.waitForLoadState('networkidle')
    note('被停用者開 /orgs/id 畫面：' + (await vp.locator('main').innerText()).replace(/\n+/g, ' / ').slice(0, 160))
    const sel = await row.getByRole('button', { name: '啟用' }).click(); await sleep(1200)
    must(sql(`SELECT disabled_at IS NULL FROM org_members WHERE org_id='${S.orgId}' AND user_id='${S.volId}'`) === 't', '重新啟用失敗')
    note('沒有「移除成員」：只有停用（資料保留）；移除/離開組織無 UI')
  }, { page: pages.owner })

  await run.step('O-35', '最後一位 owner 保護：自己降級被擋、自己沒有停用鈕；升協調員為 owner 後才能降原 owner', async ({ note }) => {
    const p = pages.owner
    const row = p.locator(`[data-testid="member-${S.ownerId}"]`)
    must(await row.getByRole('button', { name: '停用' }).count() === 0, '不應顯示對自己的停用鈕')
    await row.getByLabel('角色').selectOption('coordinator')
    const a = await p.getByRole('alert').first().innerText({ timeout: 8000 })
    note('自己降級：「' + a + '」')
    await p.reload(); await p.waitForLoadState('networkidle')
    const cid = (await meOf(pages.coord.api)).id
    S.coordId = cid
    await p.locator(`[data-testid="member-${cid}"]`).getByLabel('角色').selectOption('owner'); await sleep(1000)
    await p.locator(`[data-testid="member-${S.ownerId}"]`).getByLabel('角色').selectOption('coordinator'); await sleep(1200)
    note('原 owner 降級後 DB 角色=' + sql(`SELECT role FROM org_members WHERE org_id='${S.orgId}' AND user_id='${S.ownerId}'`))
    note('降級後原承辦人的「成員與邀請」入口：')
    await p.goto(`/orgs/${S.orgId}`); await p.waitForLoadState('networkidle')
    note('  link 數=' + await p.getByRole('link', { name: '成員與邀請' }).count())
    // 還原：由新 owner（原協調員）把原承辦人升回 owner（走 API，登入身分為協調員）
    const r = await pages.coord.api.patch(`orgs/${S.orgId}/members/${S.ownerId}`, { data: { role: 'owner' } })
    await pages.coord.api.patch(`orgs/${S.orgId}/members/${cid}`, { data: { role: 'coordinator' } })
    note(`（API，以新 owner 身分）還原原承辦人為 owner → ${r.status()}；協調員降回 coordinator`)
    must(sql(`SELECT role FROM org_members WHERE org_id='${S.orgId}' AND user_id='${S.ownerId}'`) === 'owner', '還原失敗')
    note('提醒：頁面上看到 /me 角色可能在重新整理前仍是舊值；owner 轉移無「轉讓擁有者」單一動作')
  }, { page: pages.owner })

  await run.step('O-36', '最後一位 owner 離開（刪除帳號）：據點移交、最早加入的協調員升 owner；沒有人可接則關閉（以拋棄式組織驗證）', async ({ note }) => {
    // 帳號刪除無 UI（settings 頁沒有入口），只能走 DELETE /me；組織以 SQL 種一個已審核組織（核准流程已在 O-21 走過）
    const owner2 = await registerApi({ name: `離任者${U.slice(0, 3)}`, tag: 'orgleave' })
    const co2 = await registerApi({ name: `接手者${U.slice(0, 3)}`, tag: 'orgheir' })
    const o2id = (await meOf(owner2.api)).id, c2id = (await meOf(co2.api)).id
    const org2 = sql(`INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, rep_name, contact_name, contact_phone_enc, public_email, verification_status, verified_at, created_by)
      VALUES (substr(md5(random()::text),1,10), '離任測試 ${U}', '離任測試 ${U}', 'association', '${taxId()}', 'R-1', '內政部', '王', '王', '\\x00', 'o@example.com', 'verified', now(), '${o2id}') RETURNING id`).split('\n')[0]
    sql(`INSERT INTO org_members (org_id,user_id,role,created_at) VALUES ('${org2}','${o2id}','owner', now() - interval '2 days'), ('${org2}','${c2id}','coordinator', now() - interval '1 day')`)
    const r = await owner2.api.delete('me', { data: { confirm: 'DELETE', password: PASSWORD } })
    note(`DELETE /me → ${r.status()} ${(await r.text()).slice(0, 160)}`)
    await sleep(500)
    const roles = sql(`SELECT user_id::text||':'||role::text||':'||(disabled_at IS NOT NULL) FROM org_members WHERE org_id='${org2}' ORDER BY created_at`)
    note('刪除後成員：' + roles.replace(/\n/g, ' | '))
    must(r.status() < 300, 'DELETE /me 失敗 ' + r.status())
    must(sql(`SELECT role FROM org_members WHERE org_id='${org2}' AND user_id='${c2id}'`) === 'owner', '協調員沒有升為 owner')
    gap(false, '「離開組織」沒有任何 UI；唯一路徑是刪除整個帳號（且帳號刪除本身也沒有 UI，只有 DELETE /me）。想離開組織的協調員/志工只能請 owner 停用')
  }, { page: null })

  // =========================================================== F. 事件與據點
  S.evName = `QA 花蓮地震 ${U}`
  await run.step('O-37', '承辦人建立救災事件：選縣市＋日期 → 查重無結果 → 填表建立', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/orgs/${S.orgId}/events/new`); await p.waitForLoadState('networkidle')
    await p.locator('select[name=county]').selectOption('花蓮縣')
    await p.locator('input[name=occurred_on]').fill(S.evDate)
    await p.locator('input[name=name]').waitFor({ timeout: 8000 })
    note('日期 ' + S.evDate + '（隨機歷史日期以避免撞到其他 agent 的事件）；查重無結果→直接顯示表單')
    await p.locator('input[name=name]').fill(S.evName)
    await p.locator('select[name=hazard_type]').selectOption('earthquake')
    await p.getByLabel('地區說明').fill('花蓮市、吉安鄉')
    await p.getByLabel('事件說明').fill('QA 測試事件')
    await p.getByRole('checkbox', { name: '台東縣' }).check().catch(() => {})
    await p.getByRole('button', { name: '建立事件' }).click()
    await p.waitForURL(new RegExp(`/orgs/${S.orgId}$`), { timeout: 8000 })
    await p.getByText(S.evName).waitFor()
    S.evId = sql(`SELECT id FROM relief_events WHERE name='${S.evName}'`)
    S.evSlug = sql(`SELECT slug FROM relief_events WHERE name='${S.evName}'`)
    note(`事件 id=${S.evId} slug=${S.evSlug} counties=` + sql(`SELECT counties::text FROM relief_events WHERE id='${S.evId}'`))
  }, { page: pages.owner })

  await run.step('O-38', '協調員查重：同縣市日期±7 天 → 出現「是不是這個？」→ 選「用這個事件建立據點」帶 event 參數', async ({ note }) => {
    if (!S.evId) throw new Blocked('O-37 失敗')
    const cp = pages.coordPage
    await cp.goto(`/orgs/${S.orgId}/events/new`); await cp.waitForLoadState('networkidle')
    await cp.locator('select[name=county]').selectOption('花蓮縣')
    await cp.locator('input[name=occurred_on]').fill(addDays(S.evDate, 5))
    await cp.getByText('是不是這個？').waitFor({ timeout: 8000 })
    const t = await cp.locator('.fm-sim').innerText()
    note('提示內容：' + t.replace(/\n/g, ' / ').slice(0, 200))
    must(t.includes(S.evName), '沒列出相似事件')
    await cp.getByRole('link', { name: '用這個事件建立據點' }).first().click()
    await cp.waitForURL(/sites\/new\?event=/)
    note('導向 ' + cp.url().replace(S.evId, '<eventId>'))
    await sleep(800)
    const sel = await cp.locator('select[name=event_id]').inputValue()
    must(sel === S.evId, '事件沒有預選')
    // 超出 ±7 天則不提示
    await cp.goto(`/orgs/${S.orgId}/events/new`); await cp.waitForLoadState('networkidle')
    await cp.locator('select[name=county]').selectOption('花蓮縣')
    await cp.locator('input[name=occurred_on]').fill(addDays(S.evDate, 9))
    await sleep(1500)
    note('+9 天不提示相似：' + ((await cp.getByText('是不是這個？').count()) === 0))
  }, { page: pages.coordPage })

  await run.step('O-39', '「都不是，建立新事件」路徑（協調員另建第二個事件，供後續下架測試）', async ({ note }) => {
    const cp = pages.coordPage
    S.evName2 = `QA 花蓮地震二 ${U}`
    S.evDate2 = S.evDate
    await cp.goto(`/orgs/${S.orgId}/events/new`); await cp.waitForLoadState('networkidle')
    await cp.locator('select[name=county]').selectOption('花蓮縣')
    await cp.locator('input[name=occurred_on]').fill(S.evDate)
    await cp.getByText('是不是這個？').waitFor({ timeout: 8000 })
    await cp.getByRole('button', { name: '都不是，建立新事件' }).click()
    await cp.locator('input[name=name]').fill(S.evName2)
    await cp.getByRole('button', { name: '建立事件' }).click()
    await cp.waitForURL(new RegExp(`/orgs/${S.orgId}$`), { timeout: 8000 })
    S.evId2 = sql(`SELECT id FROM relief_events WHERE name='${S.evName2}'`)
    S.evSlug2 = sql(`SELECT slug FROM relief_events WHERE name='${S.evName2}'`)
    note('第二事件 slug=' + S.evSlug2)
    // 事件編輯：UI 沒入口 → API
    const r = await pages.coord.api.patch(`relief-events/${S.evId2}`, { data: { description: '協調員改說明' } })
    note(`事件編輯（API，UI 無入口）→ ${r.status()}`)
    const outsider = await pages.out.api.patch(`relief-events/${S.evId}`, { data: { description: '路人亂改' } })
    note(`非成員改事件 → ${outsider.status()} ${(await outsider.json().catch(() => ({}))).code ?? ''}`)
    gap(false, '事件「編輯」沒有 UI（事件列表只是純文字，沒有編輯／檢視連結；PATCH /relief-events/{id} 僅 API 可用）；建立後發現名稱打錯只能請工程處理')
  }, { page: pages.coordPage })

  await run.step('O-40', '建立據點：選事件與許可、地址／收件人／電話／保留時數 → 進清單編輯頁', async ({ note }) => {
    const p = pages.owner
    S.siteTitle = `QA 花蓮物資站 ${U}`
    await p.goto(`/orgs/${S.orgId}/sites/new`); await p.waitForLoadState('networkidle')
    await pickOpt(p.locator('select[name=event_id]'), new RegExp(S.evName))
    const permitOpts = await p.locator('select[name=permit_id] option').allInnerTexts()
    note('可選許可：' + JSON.stringify(permitOpts))
    await pickOpt(p.locator('select[name=permit_id]'), new RegExp(S.permitNo))
    await p.locator('input[name=title]').fill(S.siteTitle)
    await p.locator('input[name=site_address]').fill('花蓮縣花蓮市中山路 100 號（QA 集散站）')
    await p.locator('input[name=recipient_name]').fill('林站長')
    await p.locator('input[name=recipient_phone]').fill('0912-345-678')
    await p.getByRole('button', { name: '建立據點' }).click()
    await p.waitForURL(/\/lists\/[0-9a-f-]+\/edit$/, { timeout: 10000 })
    S.siteId = p.url().match(/lists\/([0-9a-f-]+)\/edit/)[1]
    S.siteSlug = sql(`SELECT slug FROM wishlists WHERE id='${S.siteId}'`)
    note(`據點 id=${S.siteId} slug=${S.siteSlug} status=` + sql(`SELECT status||'/'||site_status||'/'||visibility FROM wishlists WHERE id='${S.siteId}'`))
    const geoCols = sql(`SELECT count(*) FROM information_schema.columns WHERE table_name='wishlists' AND (column_name ILIKE '%lat%' OR column_name ILIKE '%lng%' OR column_name ILIKE '%geo%' OR column_name ILIKE '%location%')`)
    note('據點座標欄位數（wishlists 表）=' + geoCols + '：據點只有文字地址，沒有座標或地圖（規格 §2.1 寫「帶 PostGIS 座標、地址」）')
  }, { page: pages.owner })

  await run.step('O-41', '品項（數量制）：飲水 急缺 20 箱', async ({ note }) => {
    const p = pages.owner
    await p.getByRole('button', { name: /新增品項/ }).click()
    await p.getByLabel('名稱（必填）').fill('瓶裝水 600ml（箱）')
    await p.locator('select[name=category]').selectOption('water')
    await p.locator('select[name=urgency]').selectOption('critical')
    await p.getByLabel('需要數量').fill('20')
    await p.getByRole('button', { name: '儲存品項' }).click()
    await p.getByText('瓶裝水 600ml（箱）').waitFor({ timeout: 8000 })
    S.itemQ = sql(`SELECT id FROM wishlist_items WHERE wishlist_id='${S.siteId}' AND title like '瓶裝水%'`)
    note('item id=' + S.itemQ)
    // 驗證：未選類別無法存
    await p.getByRole('button', { name: /新增品項/ }).click()
    await p.getByLabel('名稱（必填）').fill('缺類別測試')
    await p.getByRole('button', { name: '儲存品項' }).click()
    await sleep(600)
    note('未選類別/急迫程度：select required 擋住=' + (await p.locator('select[name=category]').evaluate(e => !e.checkValidity())))
    await p.getByRole('dialog').getByRole('button', { name: '取消' }).click()
  }, { page: pages.owner })

  await run.step('O-42', '品項（眾籌制）：醫療 高 睡袋 目標 1500 點，需先有收件資訊', async ({ note }) => {
    const p = pages.owner
    await p.getByRole('button', { name: /新增品項/ }).click()
    await p.getByLabel('名稱（必填）').fill('保暖睡袋')
    await p.locator('select[name=category]').selectOption('shelter')
    await p.locator('select[name=urgency]').selectOption('high')
    await p.getByLabel('心願眾籌').check()
    await p.getByLabel('目標點數').fill('1500')
    await p.getByLabel('商品參考價格').fill('1500')
    await p.getByRole('button', { name: '儲存品項' }).click()
    await sleep(1200)
    const need = (await p.getByRole('alert').count()) ? await p.getByRole('alert').first().innerText() : ''
    note('未填募集截止：「' + need + '」（標籤寫「選填」，但救災據點沒有活動日 → 實際必填）')
    S.deadlineGap = /截止|活動日/.test(need)
    await p.getByLabel('募集截止').fill(`${addDays(today(), 30)}T23:59`)
    await p.getByRole('button', { name: '儲存品項' }).click()
    await p.getByText('保暖睡袋').first().waitFor({ timeout: 8000 })
    S.itemC = sql(`SELECT id FROM wishlist_items WHERE wishlist_id='${S.siteId}' AND title='保暖睡袋'`)
    note('眾籌 item id=' + S.itemC + ' mode=' + sql(`SELECT funding_mode||'/'||target_points FROM wishlist_items WHERE id='${S.itemC}'`))
    gap(!S.deadlineGap, '眾籌品項的「募集截止」標示（選填），但救災據點沒有活動日，後端要求必填（錯誤訊息「請填寫截止時間，或先設定清單的活動日期」對 relief 據點沒有「活動日」可設，訊息誤導）；且眾籌需要在據點層級提供預設截止日')
  }, { page: pages.owner })

  await run.step('O-43', '據點上線：按「發佈並分享」→ 取得分享連結；草稿期公開頁 404', async ({ note }) => {
    const p = pages.owner
    const pre = await pages.guest.goto(`/s/${S.siteSlug}`)
    note('發佈前 /s/slug HTTP ' + pre.status())
    await p.getByRole('button', { name: '發佈並分享' }).click()
    await p.locator('#c-share-input').waitFor({ timeout: 8000 })
    const share = await p.locator('#c-share-input').inputValue()
    note('分享連結=' + share)
    must(sql(`SELECT status FROM wishlists WHERE id='${S.siteId}'`) === 'active', '未 active')
    await p.keyboard.press('Escape')
  }, { page: pages.owner })

  await run.step('O-44', '公開曝光：/e 列表、/e/[slug]（據點、缺口彙總）、/o/[slug]（據點出現）、/s/[slug]（徽章、地址、不顯示捐助者）', async ({ note }) => {
    const p = pages.guest
    await p.goto('/e'); await p.waitForLoadState('networkidle')
    const el = await p.locator('main').innerText()
    must(el.includes(S.evName), '/e 沒列出事件')
    await p.goto(`/e/${S.evSlug}`); await p.waitForLoadState('networkidle')
    const et = await p.locator('main').innerText()
    note('/e/slug：' + et.replace(/\n+/g, ' / ').slice(0, 260))
    must(et.includes(S.siteTitle) && et.includes('急缺'), '事件頁缺據點或急缺彙總')
    await p.goto(`/o/${S.orgSlug}`); await p.waitForLoadState('networkidle')
    must((await p.locator('main').innerText()).includes(S.siteTitle), '/o 沒列據點')
    await p.screenshot({ path: resolve(OUT, 'org', 'O-44-org-public.png'), fullPage: true })
    await p.goto(`/s/${S.siteSlug}`); await p.waitForLoadState('networkidle')
    const st = await p.locator('main').innerText()
    note('/s/slug：' + st.replace(/\n+/g, ' / ').slice(0, 320))
    must(st.includes('已驗證組織') && st.includes('花蓮市中山路'), '據點頁缺徽章或地址')
    must(!st.includes('0912'), '據點頁洩漏收件電話？')
    note('收件電話是否公開顯示=' + st.includes('0912-345-678') + '；收件人姓名是否顯示=' + st.includes('林站長'))
  }, { page: pages.guest })

  await run.step('O-45', '探索頁 /explore 不列救災據點、頂端有連到 /e 的橫幅', async ({ note }) => {
    const p = pages.guest
    const r = await p.goto('/explore'); await p.waitForLoadState('networkidle')
    note('HTTP ' + r.status() + ' url=' + p.url())
    const t = await p.locator('main').innerText()
    must(!t.includes(S.siteTitle), '探索頁列出了救災據點')
    note('橫幅：' + (await p.getByRole('link', { name: /災害救援/ }).count() ? '有' : '無'))
  }, { page: pages.guest })

  // =========================================================== G. 外部捐助者
  pages.donor = await registerApi({ name: `愛心捐助者${U.slice(0, 3)}`, tag: 'orgdonor' })
  pages.donorPage = await newPage({ user: pages.donor })
  await run.step('O-46', '外部捐助者（未登入）在據點頁按「我來送」→ 要求登入（D50）', async ({ note }) => {
    const p = pages.guest
    await p.goto(`/s/${S.siteSlug}`); await p.waitForLoadState('networkidle')
    const lg = p.getByText('登入後來送').first()
    note('未登入時品項上的動作：「' + (await lg.innerText()) + '」（元素=' + (await lg.evaluate(e => e.tagName.toLowerCase())) + '）')
    await lg.click()
    await p.waitForURL(/\/login/, { timeout: 8000 }).catch(() => {})
    note('url=' + p.url())
    must(/\/login/.test(p.url()), '未導向登入')
    must(/redirect/.test(p.url()), '登入後沒有帶回據點的 redirect')
  }, { page: pages.guest })

  await run.step('O-47', '外部捐助者登入後認捐 3 箱（數量制）', async ({ note }) => {
    const p = pages.donorPage
    await p.goto(`/s/${S.siteSlug}`); await p.waitForLoadState('networkidle')
    await p.getByRole('button', { name: '我來送' }).first().click()
    const inc = p.getByRole('button', { name: '增加' })
    await inc.click(); await inc.click()
    await p.getByRole('button', { name: '確定要送' }).click()
    await sleep(2000)
    const c = sql(`SELECT qty||':'||status FROM claims WHERE item_id='${S.itemQ}' AND user_id='${(await meOf(pages.donor.api)).id}'`)
    note('claims：' + c)
    must(c.startsWith('3:'), '認捐數量不是 3：' + c)
    S.claimExpires = sql(`SELECT expires_at - created_at FROM claims WHERE item_id='${S.itemQ}'`)
    note('保留期限（預設 48h）：' + S.claimExpires)
    note('捐助者頁面訊息：' + (await p.locator('[role=status], .g-toast, .g-ok').allInnerTexts()).join('|'))
  }, { page: pages.donorPage })

  await run.step('O-48', '眾籌品項：捐助者貢獻點數（點數由 staff 發放：平台外動作）→ 進度條', async ({ note }) => {
    const p = pages.donorPage
    const did = (await meOf(pages.donor.api)).id
    const g = await pages.staffUser.api.post('admin/wallets/grants', { data: { user_id: did, points: 2000, reason: 'QA 組織流程' }, headers: { 'Idempotency-Key': randomUUID() } })
    note(`（API）staff 發點 → ${g.status()} ${g.ok() ? '' : (await g.text()).slice(0, 160)}`)
    if (!g.ok()) throw new Blocked('發點失敗：' + g.status())
    await p.goto(`/s/${S.siteSlug}`); await p.waitForLoadState('networkidle')
    await p.getByRole('button', { name: '心願眾籌' }).first().click()
    await p.getByRole('dialog').getByRole('spinbutton').first().fill('600').catch(() => {})
    await p.screenshot({ path: resolve(OUT, 'org', 'O-48-fund-sheet.png') })
    const btns = await p.getByRole('dialog').getByRole('button').allInnerTexts()
    note('眾籌對話按鈕：' + JSON.stringify(btns))
    const go = p.getByRole('dialog').getByRole('button', { name: /確定|贊助|投入|送出/ }).last()
    await go.click(); await sleep(2500)
    note('contributions：' + sql(`SELECT points||':'||status FROM contributions WHERE item_id='${S.itemC}'`) + '；pledged=' + sql(`SELECT pledged_points FROM wishlist_items WHERE id='${S.itemC}'`))
  }, { page: pages.donorPage })

  await run.step('O-49', '組織端看到認捐／眾籌進度：誰認捐、數量、保留期限；有無感謝／通知', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/lists/${S.siteId}/edit`); await p.waitForLoadState('networkidle')
    const t = await p.locator('main').innerText()
    note('編輯頁品項區：' + t.replace(/\n+/g, ' / ').slice(0, 420))
    note('品項區是否顯示「已認領」數量（在途 3 箱）=' + /已認領|認領/.test(t.split('據點')[0]) + '；顯示已實收=' + t.includes('已實收'))
    S.noClaimedOnEdit = !/已認領/.test(t.split('據點')[0])
    const names = t.includes('愛心捐助者')
    note('組織看得到捐助者名稱=' + names)
    const notif = sql(`SELECT kind||'→'||status FROM notifications WHERE payload::text LIKE '%${S.siteId}%' ORDER BY created_at`)
    note('與此據點相關的通知：' + (notif || '(無)').replace(/\n/g, ' | '))
    gap(!S.noClaimedOnEdit, '組織編輯頁品項只顯示「需要 20」與「已實收」，不顯示已被認捐（在途）的數量與保留期限（lists/[id]/edit.vue:172-174 只用 qty_needed / qty_received，qty_claimed 沒有顯示）；組織端沒有「誰認捐了什麼」的清單、也沒有捐助者聯絡或感謝管道；relief 據點強制匿名（設計如此），但組織無法對捐助者回報收到/致謝，亦無認捐到貨的追蹤（見「缺少功能」）')
  }, { page: pages.owner })

  // =========================================================== H. 實收
  await run.step('O-50', '志工登記實收：/orgs/[id]/sites/[wid]/receipts → 登記 5 箱（備註）→ 累計與經手人', async ({ note }) => {
    const vp = pages.volPage
    await vp.goto(`/orgs/${S.orgId}/sites/${S.siteId}/receipts`); await vp.waitForLoadState('networkidle')
    await vp.getByText('瓶裝水 600ml（箱）').waitFor({ timeout: 8000 })
    const card = vp.locator('li.c-card', { hasText: '瓶裝水' })
    note('登記前：' + (await card.innerText()).replace(/\n/g, ' / '))
    await card.getByRole('button', { name: '登記實收' }).click()
    await vp.getByRole('dialog').getByLabel('實收數量').fill('5')
    await vp.getByRole('dialog').getByLabel('備註').fill('第一車，志工點收')
    await vp.getByRole('dialog').getByRole('button', { name: '登記', exact: true }).click()
    await sleep(1500)
    const after = (await card.innerText()).replace(/\n/g, ' / ')
    note('登記後：' + after)
    must(after.includes('累計實收 5'), '累計不是 5')
    await card.getByRole('button', { name: '登記實收' }).click()
    const hist = await vp.getByRole('dialog').innerText()
    note('對話內歷史：' + hist.replace(/\n/g, ' / ').slice(0, 160))
    await vp.getByRole('dialog').getByRole('button', { name: '關閉' }).click().catch(() => vp.keyboard.press('Escape'))
  }, { page: pages.volPage })

  await run.step('O-51', '實收邊界：0／負數／超大值（API）、同 Idempotency-Key 重放不重複；實收 > 需求 20 的處理', async ({ note }) => {
    const base = { item_id: S.itemQ }
    for (const qty of [0, -3, 100001]) {
      const r = await S.volApi.post(`orgs/${S.orgId}/receipts`, { data: { ...base, qty }, headers: { 'Idempotency-Key': randomUUID() } })
      note(`qty=${qty} → ${r.status()} ${(await r.json().catch(() => ({}))).code ?? ''}`)
      must(r.status() === 422 || r.status() === 400, `qty=${qty} 應被擋`)
    }
    const key = randomUUID()
    const a = await S.volApi.post(`orgs/${S.orgId}/receipts`, { data: { ...base, qty: 2 }, headers: { 'Idempotency-Key': key } })
    const b = await S.volApi.post(`orgs/${S.orgId}/receipts`, { data: { ...base, qty: 2 }, headers: { 'Idempotency-Key': key } })
    note(`同 key 兩次 → ${a.status()} / ${b.status()} replayed=${b.headers()['idempotency-replayed']}`)
    must(b.headers()['idempotency-replayed'] === 'true', '未標示 replayed')
    const tot = sql(`SELECT sum(qty) FROM receipts WHERE item_id='${S.itemQ}'`)
    note('累計=' + tot + '（5+2）')
    const over = await S.volApi.post(`orgs/${S.orgId}/receipts`, { data: { ...base, qty: 50 }, headers: { 'Idempotency-Key': randomUUID() } })
    note(`實收 50（累計超過需求 20、超過已認捐 3）→ ${over.status()}：系統不限制`)
    note('沒有「更正／作廢」實收的端點或 UI：登錯只能再登一筆（receipts 只增不減，無負數）')
    gap(false, '實收一旦登錯（例如多打一個 0）無法更正或作廢，也沒有與認捐量對帳的提示（實收 57 > 需求 20 無警示）')
  }, { page: pages.volPage })

  await run.step('O-52', '志工的權限邊界（UI＋API）：看不到新增事件/據點/成員；直接打管理 API 被拒', async ({ note }) => {
    const vp = pages.volPage
    await vp.goto(`/orgs/${S.orgId}`); await vp.waitForLoadState('networkidle')
    const links = { 新增事件: await vp.getByRole('link', { name: '新增事件' }).count(), 新增據點: await vp.getByRole('link', { name: '新增據點' }).count(), 成員與邀請: await vp.getByRole('link', { name: '成員與邀請' }).count(), 編輯: await vp.getByRole('link', { name: '編輯' }).count(), 實收登記: await vp.getByRole('link', { name: '實收登記' }).count() }
    note('志工可見連結數：' + JSON.stringify(links))
    must(links.新增事件 === 0 && links.新增據點 === 0 && links.成員與邀請 === 0 && links.編輯 === 0 && links.實收登記 > 0, 'UI 權限顯示不符')
    const tries = {
      建事件: await S.volApi.post(`orgs/${S.orgId}/events`, { data: { name: 'x', hazard_type: 'flood', counties: ['花蓮縣'], occurred_on: today() } }),
      建據點: await S.volApi.post(`orgs/${S.orgId}/sites`, { data: { title: 'x', event_id: S.evId, permit_id: S.p1, site_address: '花蓮縣某處一號', recipient_name: 'a', recipient_phone: '0912345678' } }),
      邀請: await S.volApi.post(`orgs/${S.orgId}/invites`, { data: { email: 'a@b.co', role: 'volunteer' } }),
      改角色: await S.volApi.patch(`orgs/${S.orgId}/members/${S.ownerId}`, { data: { role: 'volunteer' } }),
      改據點狀態: await S.volApi.patch(`wishlists/${S.siteId}`, { data: { site_status: 'paused' } }),
      加品項: await S.volApi.post(`wishlists/${S.siteId}/items`, { data: { title: 'x', qty_needed: 1, category: 'water', urgency: 'low' } }),
      看成員: await S.volApi.get(`orgs/${S.orgId}/members`),
      看邀請: await S.volApi.get(`orgs/${S.orgId}/invites`),
    }
    const out = {}
    for (const [k, r] of Object.entries(tries)) out[k] = r.status()
    note('志工打管理 API：' + JSON.stringify(out))
    for (const k of ['建事件', '建據點', '邀請', '改角色', '改據點狀態', '加品項', '看邀請']) must([403, 404].includes(out[k]), `${k} 應被拒，實際 ${out[k]}`)
    note('志工 GET members=' + out.看成員 + '（規格：volunteer 可看儀表板）')
    await vp.goto(`/lists/${S.siteId}/edit`); await vp.waitForLoadState('networkidle'); await sleep(800)
    const vt = (await vp.locator('main').innerText()).replace(/\n+/g, ' / ')
    note('志工直接開 /lists/id/edit：' + vt.slice(vt.indexOf('LISTS'), vt.indexOf('LISTS') + 200))
    note('志工在編輯頁可見控制項：site_status 下拉=' + await vp.locator('select[name=site_status]').count() + '、發佈鈕=' + await vp.getByRole('button', { name: /發佈/ }).count() + '、新增品項=' + await vp.getByRole('button', { name: /新增品項/ }).count() + '、刪除=' + await vp.getByRole('button', { name: '刪除' }).count())
    await vp.screenshot({ path: resolve(OUT, 'org', 'O-52-vol-edit.png'), fullPage: true })
    await vp.goto(`/orgs/${S.orgId}/members`); await vp.waitForLoadState('networkidle')
    note('志工開 /orgs/id/members：邀請表單數=' + await vp.locator('form.m-form').count())
  }, { page: pages.volPage })

  // =========================================================== I. 營運狀態
  await run.step('O-53', '暫停據點 → 公開頁顯示暫停且無法認捐、/e 不再列出 → 恢復', async ({ note }) => {
    const p = pages.owner
    await p.goto(`/lists/${S.siteId}/edit`); await p.waitForLoadState('networkidle')
    await p.locator('select[name=site_status]').selectOption('paused'); await sleep(1500)
    must(sql(`SELECT site_status FROM wishlists WHERE id='${S.siteId}'`) === 'paused', '未暫停')
    const g = pages.guest
    await g.goto(`/s/${S.siteSlug}`); await g.waitForLoadState('networkidle')
    const t = await g.locator('main').innerText()
    note('暫停後 /s：' + t.replace(/\n+/g, ' / ').slice(0, 160))
    must(t.includes('暫停收件'), '公開頁沒顯示暫停')
    const sendBtns = g.getByRole('button', { name: /來送|已暫停|暫停/ })
    note('暫停時送禮按鈕：數量=' + await sendBtns.count() + ' 文字=' + JSON.stringify(await sendBtns.allInnerTexts()) + ' disabled=' + JSON.stringify(await sendBtns.evaluateAll(els => els.map(e => e.disabled))))
    const did = (await meOf(pages.donor.api)).id
    S.claimId = sql(`SELECT id FROM claims WHERE item_id='${S.itemQ}' AND user_id='${did}' AND status='reserved'`)
    const chg = await pages.donor.api.patch(`claims/${S.claimId}`, { data: { qty: 2 }, headers: { 'Idempotency-Key': randomUUID() } })
    note(`暫停期間既有認捐者改數量（API）→ ${chg.status()} ${(await chg.json().catch(() => ({}))).code ?? ''}`)
    await g.goto(`/e/${S.evSlug}`); await g.waitForLoadState('networkidle')
    note('暫停後 /e/slug 仍列據點=' + (await g.locator('main').innerText()).includes(S.siteTitle))
    const claim = await pages.donor.api.post(`items/${S.itemQ}/claims`, { data: { qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } })
    note(`暫停期間 API 認捐 → ${claim.status()} ${(await claim.json().catch(() => ({}))).code ?? ''}`)
    must(claim.status() >= 400, '暫停期間仍可認捐')
    note('已認捐者（O-47 的 3 箱）在暫停時是否能修改/取消：見畫面')
    await p.locator('select[name=site_status]').selectOption('active'); await sleep(1500)
    must(sql(`SELECT site_status FROM wishlists WHERE id='${S.siteId}'`) === 'active', '未恢復')
  }, { page: pages.owner })

  await run.step('O-54', '結束據點（closed）→ 實收登記鎖定；能否由 closed 回到 active', async ({ note }) => {
    const p = pages.owner
    await p.locator('select[name=site_status]').selectOption('closed'); await sleep(1500)
    const vp = pages.volPage
    await vp.goto(`/orgs/${S.orgId}/sites/${S.siteId}/receipts`); await vp.waitForLoadState('networkidle')
    const dis = await vp.getByRole('button', { name: '登記實收' }).first().isDisabled()
    note('closed 時「登記實收」disabled=' + dis)
    const r = await S.volApi.post(`orgs/${S.orgId}/receipts`, { data: { item_id: S.itemQ, qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } })
    note(`closed 時 API 實收 → ${r.status()} ${(await r.json().catch(() => ({}))).code ?? ''}`)
    await p.locator('select[name=site_status]').selectOption('active'); await sleep(1500)
    note('closed → active 後 site_status=' + sql(`SELECT site_status FROM wishlists WHERE id='${S.siteId}'`))
    note('提醒：closed 之後仍可點到貨物（還在路上的 3 箱認捐）卻再也不能登記實收')
  }, { page: pages.owner })

  await run.step('O-55', '許可到期（SQL 快轉）：到期當下據點仍上線？重新發布/恢復被擋？背景 tick 何時處理', async ({ note }) => {
    // 建立使用第二張許可的第二個據點（草稿→上線），再把該許可 valid_to 快轉到昨天
    const cp = pages.coordPage
    S.site2Title = `QA 吉安分站 ${U}`
    await cp.goto(`/orgs/${S.orgId}/sites/new`); await cp.waitForLoadState('networkidle')
    await pickOpt(cp.locator('select[name=event_id]'), new RegExp(S.evName2))
    await pickOpt(cp.locator('select[name=permit_id]'), new RegExp(S.permitNo2))
    await cp.locator('input[name=title]').fill(S.site2Title)
    await cp.locator('input[name=site_address]').fill('花蓮縣吉安鄉中興路 8 號（QA 分站）')
    await cp.locator('input[name=recipient_name]').fill('黃站長')
    await cp.locator('input[name=recipient_phone]').fill('0922-333-444')
    await cp.getByRole('button', { name: '建立據點' }).click()
    await cp.waitForURL(/\/lists\/[0-9a-f-]+\/edit$/, { timeout: 10000 })
    S.site2Id = cp.url().match(/lists\/([0-9a-f-]+)\/edit/)[1]
    S.site2Slug = sql(`SELECT slug FROM wishlists WHERE id='${S.site2Id}'`)
    await cp.getByRole('button', { name: /新增品項/ }).click()
    await cp.getByLabel('名稱（必填）').fill('毛毯')
    await cp.locator('select[name=category]').selectOption('shelter'); await cp.locator('select[name=urgency]').selectOption('medium')
    await cp.getByLabel('需要數量').fill('10'); await cp.getByRole('button', { name: '儲存品項' }).click()
    await cp.getByText('毛毯').first().waitFor({ timeout: 8000 })
    await cp.getByRole('button', { name: '發佈並分享' }).click()
    await cp.locator('#c-share-input').waitFor({ timeout: 8000 }); await cp.keyboard.press('Escape')
    must(sql(`SELECT status||'/'||site_status FROM wishlists WHERE id='${S.site2Id}'`) === 'active/active', '分站未上線')
    // 另鋪一個同許可的乙站（API，流程同上）專門測「許可過期後恢復營運」；甲站維持上線，留給背景 tick 測試（org-tick.mjs）
    const w2 = await (await pages.coord.api.post(`orgs/${S.orgId}/sites`, { data: { title: `QA 吉安分站乙 ${U}`, event_id: S.evId2, permit_id: S.p2, site_address: '花蓮縣吉安鄉中興路 9 號', recipient_name: '黃', recipient_phone: '0922333445' } })).json()
    S.site2bId = w2.id
    await pages.coord.api.post(`wishlists/${w2.id}/items`, { data: { title: '睡墊', qty_needed: 5, category: 'shelter', urgency: 'low' } })
    must((await pages.coord.api.patch(`wishlists/${w2.id}`, { data: { status: 'active' } })).ok(), '乙站上線失敗')
    sql(`UPDATE org_permits SET valid_to = ((now() AT TIME ZONE 'Asia/Taipei')::date - 1) WHERE id='${S.p2}'`)
    note('（SQL）把第二張許可 valid_to 快轉到昨天 — 平台外時間快轉')
    const g = pages.guest
    await g.goto(`/s/${S.site2Slug}`); await g.waitForLoadState('networkidle')
    note('到期後、背景 tick 前，公開頁送禮按鈕=' + JSON.stringify(await g.getByRole('button', { name: /來送/ }).allInnerTexts()) + '；/e/slug 仍列此據點=' + await (async () => { await g.goto(`/e/${S.evSlug2}`); return (await g.locator('main').innerText()).includes(S.site2Title) })())
    const item2 = sql(`SELECT id FROM wishlist_items WHERE wishlist_id='${S.site2Id}'`)
    const dc = await pages.donor.api.post(`items/${item2}/claims`, { data: { qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } })
    note(`許可已過期但 tick 尚未跑：捐助者 API 認捐 → ${dc.status()} ${(await dc.json().catch(() => ({}))).code ?? ''}`)
    S.expiredClaimStatus = dc.status()
    S.expiredAt = new Date().toISOString()
    // 嘗試在 UI 把據點狀態暫停→恢復（恢復營運會重驗許可）
    const cpp = pages.coordPage
    await cpp.goto(`/lists/${S.site2bId}/edit`); await cpp.waitForLoadState('networkidle')
    await cpp.locator('select[name=site_status]').selectOption('paused'); await sleep(1200)
    await cpp.locator('select[name=site_status]').selectOption('active'); await sleep(1500)
    const st = sql(`SELECT site_status FROM wishlists WHERE id='${S.site2bId}'`)
    const al = (await cpp.getByRole('alert').count()) ? await cpp.getByRole('alert').first().innerText() : ''
    note(`許可過期後「恢復營運」→ site_status=${st}；畫面訊息=「${al}」`)
    must(st !== 'active', '許可過期的據點仍可恢復營運')
    // 嘗試建立草稿據點並發布：新據點選用到期許可
    note('甲站（' + S.site2Id + '）維持 active 且許可已過期：背景 tick（permit_expiry_tick，每小時整點 +API 啟動分秒）尚未執行 → 由 qa/role-flows/org-tick.mjs 在下一次 tick 後驗證')
    // 專屬「許可到期 tick」測試組織：獨立的 owner／組織／據點，不受後面撤銷步驟（O-56、O-61）影響；
    // 組織以 SQL 種為 verified（核准流程已在 O-21 走過）；據點用 API 建立並發布後，再把許可 valid_to 快轉到昨天（平台外動作）
    const tickOwner = await registerApi({ name: `到期測試承辦${U.slice(0, 3)}`, tag: 'orgtick' })
    const tOwnerId = (await meOf(tickOwner.api)).id
    const tOrg = sql(`INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, rep_name, contact_name, contact_phone_enc, public_email, verification_status, verified_at, created_by)
      VALUES (substr(md5(random()::text),1,10), '到期測試 ${U}', '到期測試 ${U}', 'association', '${taxId()}', 'R-1', '內政部', '王', '王', '\\x00', 'o@example.com', 'verified', now(), '${tOwnerId}') RETURNING id`).split('\n')[0]
    sql(`INSERT INTO org_members (org_id,user_id,role) VALUES ('${tOrg}','${tOwnerId}','owner')`)
    const tPermit = sql(`INSERT INTO org_permits (org_id, permit_no, valid_from, valid_to, status) VALUES ('${tOrg}', 'T-${U}', current_date - 5, current_date + 30, 'approved') RETURNING id`).split('\n')[0]
    const tSite = await (await tickOwner.api.post(`orgs/${tOrg}/sites`, { data: { title: `QA 到期測試站 ${U}`, event_id: S.evId, permit_id: tPermit, site_address: '花蓮縣壽豐鄉某路 5 號', recipient_name: '周', recipient_phone: '0955666777' } })).json()
    await tickOwner.api.post(`wishlists/${tSite.id}/items`, { data: { title: '乾糧', qty_needed: 30, category: 'food', urgency: 'high' } })
    must((await tickOwner.api.patch(`wishlists/${tSite.id}`, { data: { status: 'active' } })).ok(), '到期測試站上線失敗')
    sql(`UPDATE org_permits SET valid_to = ((now() AT TIME ZONE 'Asia/Taipei')::date - 1) WHERE id='${tPermit}'`)
    S.tickOrgId = tOrg; S.tickSiteId = tSite.id; S.tickPermitId = tPermit; S.tickOwnerEmail = tickOwner.email; S.tickOwnerId = tOwnerId
    note(`（SQL/API）專屬到期測試組織 ${tOrg}、據點 ${tSite.id} 已上線且許可昨日到期；等整點 tick 由 org-tick.mjs 驗證，狀態=` + sql(`SELECT status||'/'||site_status FROM wishlists WHERE id='${tSite.id}'`))
  }, { page: pages.coordPage })

  await run.step('O-56', '【staff】撤銷已核准許可 P1 → 使用該許可的上線據點被暫停；組織端看到什麼', async ({ note }) => {
    // 為了保留主據點供後續使用，這裡撤銷的是 P2 對應的分站許可之外：改用第三張許可？— 直接用 P2 的分站已過期；撤銷用 P1 會影響主據點，故先新增 P3 與據點
    const d = await (await S.ownerApi.post(`orgs/${S.orgId}/documents?kind=permit`, { data: pdfBytes(1024), headers: { 'Content-Type': 'application/pdf' } })).json()
    S.permitNo3 = `勸募三號${U}`
    S.p3 = (await (await S.ownerApi.post(`orgs/${S.orgId}/permits`, { data: { permit_no: S.permitNo3, valid_from: addDays(today(), -3), valid_to: addDays(today(), 90), document_id: d.id } })).json()).id
    const sp = pages.staff
    let card = await staffOpen(sp, S.orgId, 'verified')
    const permit = card.locator(`[data-permit="${S.p3}"]`)
    await permit.getByRole('button', { name: '核准許可' }).click(); await dialogOk(sp, '核准許可')
    await permit.getByText('已核准').first().waitFor({ timeout: 8000 })
    // 協調員用 P3 建第三個據點並上線（API 快速鋪設：UI 流程同 O-40/O-55）
    const w = await (await pages.coord.api.post(`orgs/${S.orgId}/sites`, { data: { title: `QA 撤銷許可測試站 ${U}`, event_id: S.evId, permit_id: S.p3, site_address: '花蓮縣秀林鄉某路 3 號', recipient_name: '張', recipient_phone: '0933444555' } })).json()
    S.site3Id = w.id; S.site3Slug = w.slug
    await pages.coord.api.post(`wishlists/${S.site3Id}/items`, { data: { title: '口罩', qty_needed: 100, category: 'medical', urgency: 'high' } })
    const pr = await pages.coord.api.patch(`wishlists/${S.site3Id}`, { data: { status: 'active' } })
    note(`（API）鋪設 P3 據點並上線 → ${pr.status()}`)
    must(sql(`SELECT status||'/'||site_status FROM wishlists WHERE id='${S.site3Id}'`) === 'active/active', 'site3 未上線')
    // 撤銷
    card = await staffOpen(sp, S.orgId, 'verified')
    const pm = card.locator(`[data-permit="${S.p3}"]`)
    await pm.getByRole('button', { name: '撤銷許可' }).click(); await dialogOk(sp, '撤銷許可')
    await sp.getByText('已處理').first().waitFor({ timeout: 8000 })
    note('撤銷後 site3=' + sql(`SELECT status||'/'||site_status FROM wishlists WHERE id='${S.site3Id}'`) + '；主據點 site1=' + sql(`SELECT status||'/'||site_status FROM wishlists WHERE id='${S.siteId}'`))
    const cp = pages.coordPage
    await cp.goto(`/orgs/${S.orgId}`); await cp.waitForLoadState('networkidle')
    const li = await cp.locator('li', { hasText: '撤銷許可測試站' }).innerText()
    note('組織後台該據點列：' + li.replace(/\n/g, ' / '))
    const g = pages.guest
    const rr = await g.goto(`/s/${S.site3Slug}`); await g.waitForLoadState('networkidle')
    note('公開頁：HTTP ' + rr.status() + '；內容=' + (await g.locator('main').innerText()).replace(/\n+/g, ' / ').slice(0, 100))
    await sleep(3000)
    const pm3 = await waitForMail(S.ownerEmail, { subject: /已撤銷/, timeout: 30000 }).catch(() => null)
    note('owner 收到的許可撤銷信：' + (pm3 ? `「${pm3.subject}」 內文=${pm3.text.replace(/\s+/g, ' ').slice(0, 260)}` : '(無)'))
    S.pm3Text = pm3?.text ?? ''
    const coordMails = (await listMail(pages.coord.email)).map(m => m.subject)
    note('建立該據點的協調員收到的信：' + JSON.stringify(coordMails))
    const mentionsSite = /據點|暫停/.test(S.pm3Text)
    note('信中是否提到據點已被暫停=' + mentionsSite)
    gap(false, `許可撤銷信只寄給 owner${mentionsSite ? '' : '，且內文沒有說明哪些據點因此被暫停'}；建立據點的協調員沒有收到通知；組織後台據點列只顯示 paused，不說明原因（org/index.vue 只畫 site_status 徽章）`)
  }, { page: pages.coordPage })

  await run.step('O-57', '【staff】下架事件（隱藏）→ 組織端、公開頁看到什麼', async ({ note }) => {
    const sp = pages.staff
    await sp.goto('/admin/relief-events'); await sp.waitForLoadState('networkidle')
    await sp.getByLabel('以 id 或 slug 篩選').fill(S.evSlug2)
    const art = sp.locator(`article[data-event="${S.evId2}"]`)
    await art.waitFor({ timeout: 8000 }).catch(() => {})
    note('事件二是否列於 admin 事件管理列表=' + (await art.count()) + '（該列表取自 GET /public/relief-events：只含「有上線據點」的事件；沒有據點或據點都暫停的事件，staff 只能貼 UUID 操作）')
    // 事件二的據點是 O-55 的分站（已因許可過期被拒恢復為 paused）→ 公開列表不含它；改以 UUID 直接操作
    await sp.getByLabel('以 id 或 slug 篩選').fill(S.evId2)
    const d = sp.locator(`article[data-event="${S.evId2}"]`)
    await d.getByRole('button', { name: '隱藏', exact: true }).click()
    await sp.getByRole('dialog').getByLabel('原因').fill('QA 測試：重複事件')
    await dialogOk(sp, '隱藏')
    await sp.getByText('已處理').first().waitFor({ timeout: 8000 })
    note('DB moderation_status=' + sql(`SELECT moderation_status||'/'||coalesce(moderation_reason,'') FROM relief_events WHERE id='${S.evId2}'`))
    const g = pages.guest
    const r = await g.goto(`/e/${S.evSlug2}`); await g.waitForLoadState('networkidle')
    note('公開 /e/slug HTTP ' + r.status() + '：' + (await g.locator('main').innerText()).replace(/\n+/g, ' / ').slice(0, 80))
    const cp = pages.coordPage
    await cp.goto(`/orgs/${S.orgId}`); await cp.waitForLoadState('networkidle')
    const evs = (await cp.locator('.oh-sec').first().innerText()).replace(/\n/g, ' / ')
    note('組織後台事件列：' + evs)
    const hiddenShown = evs.includes(S.evName2)
    note('被下架事件仍列在組織後台=' + hiddenShown + '；是否有「已下架/原因」標示=' + /下架|隱藏|hidden/.test(evs))
    const mk = await pages.coord.api.post(`orgs/${S.orgId}/sites`, { data: { title: 'x'.repeat(5), event_id: S.evId2, permit_id: S.p1, site_address: '花蓮縣某處一號', recipient_name: 'a', recipient_phone: '0912345678' } })
    note(`在下架事件上建據點（API）→ ${mk.status()} ${(await mk.json().catch(() => ({}))).code ?? ''}`)
    const notif = sql(`SELECT count(*) FROM notifications WHERE payload::text LIKE '%${S.evId2}%'`)
    gap(false, '事件被下架時組織後台事件列沒有任何標示或原因（org/index.vue 只畫 name/日期/縣市），建立者組織也無通知（notifications 筆數=' + notif + '）；唯有 sites/new 的下拉默默把它濾掉')
  }, { page: pages.staff })

  // =========================================================== J. 權限矩陣
  await run.step('O-58', '非成員（路人）與未登入者的邊界：UI 與 API', async ({ note }) => {
    const op = pages.outPage
    await op.goto(`/orgs/${S.orgId}`); await op.waitForLoadState('networkidle')
    await sleep(800)
    const ot = (await op.locator('main').innerText()).replace(/\n+/g, ' / ')
    note('路人開 /orgs/id：' + ot.slice(ot.indexOf('ORGS')) .slice(0, 160) + ' | url=' + op.url())
    await op.goto(`/orgs/${S.orgId}/members`); await sleep(800)
    const ot2 = (await op.locator('main').innerText()).replace(/\n+/g, ' / ')
    note('路人開 members：' + ot2.slice(ot2.indexOf('ORGS')).slice(0, 160) + '；邀請表單數=' + await op.locator('form.m-form').count())
    await op.goto(`/orgs/${S.orgId}/sites/${S.siteId}/receipts`); await sleep(800)
    note('路人開 receipts：' + (await op.locator('main').innerText()).replace(/\n+/g, ' / ').split('ORGS')[1]?.slice(0, 140))
    const o = pages.out.api
    const res = {}
    for (const [k, fn] of Object.entries({
      組織資料: () => o.get(`orgs/${S.orgId}`), 成員: () => o.get(`orgs/${S.orgId}/members`), 據點: () => o.get(`orgs/${S.orgId}/sites`),
      許可: () => o.get(`orgs/${S.orgId}/permits`), 實收: () => o.get(`orgs/${S.orgId}/sites/${S.siteId}/receipts`),
      建據點: () => o.post(`orgs/${S.orgId}/sites`, { data: { title: 'x', event_id: S.evId, permit_id: S.p1, site_address: '花蓮縣某處一號', recipient_name: 'a', recipient_phone: '0912345678' } }), 邀請: () => o.post(`orgs/${S.orgId}/invites`, { data: { email: 'a@b.co', role: 'owner' } }),
      送審: () => o.post(`orgs/${S.orgId}/submit`), 登記實收: () => o.post(`orgs/${S.orgId}/receipts`, { data: { item_id: S.itemQ, qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } }),
      改據點: () => o.patch(`wishlists/${S.siteId}`, { data: { site_status: 'closed' } }), 下載文件: () => o.get(`orgs/${S.orgId}/documents/${sql(`SELECT id FROM org_documents WHERE org_id='${S.orgId}' LIMIT 1`)}`),
    })) res[k] = (await fn()).status()
    note('路人 API 狀態碼：' + JSON.stringify(res))
    for (const [k, v] of Object.entries(res)) must([403, 404].includes(v), `${k} 路人應被拒，實際 ${v}`)
    // 協調員管理類
    const c = pages.coord.api
    const cres = {
      邀請: (await c.post(`orgs/${S.orgId}/invites`, { data: { email: 'a@b.co', role: 'volunteer' } })).status(),
      改角色: (await c.patch(`orgs/${S.orgId}/members/${S.volId}`, { data: { role: 'owner' } })).status(),
      送審: (await c.post(`orgs/${S.orgId}/submit`)).status(),
      改組織資料: (await c.patch(`orgs/${S.orgId}`, { data: { intro: 'x' } })).status(),
      撤回: (await c.post(`orgs/${S.orgId}/withdraw`)).status(),
      看成員: (await c.get(`orgs/${S.orgId}/members`)).status(),
      看邀請: (await c.get(`orgs/${S.orgId}/invites`)).status(),
    }
    note('協調員 API：' + JSON.stringify(cres))
    for (const k of ['邀請', '改角色', '送審', '改組織資料', '撤回', '看邀請']) must([403, 404, 409].includes(cres[k]), `協調員 ${k} 應被拒，實際 ${cres[k]}`)
    // 未登入
    const anon = await apiCtx()
    const ar = { 組織: (await anon.get(`orgs/${S.orgId}`)).status(), 邀請預覽: (await anon.get('org-invites/abc')).status(), 公開組織: (await anon.get(`public/orgs/${S.orgSlug}`)).status() }
    note('未登入：' + JSON.stringify(ar))
    const gp = pages.guest
    await gp.goto(`/orgs/${S.orgId}`); await gp.waitForLoadState('networkidle')
    note('未登入開 /orgs/id → ' + gp.url())
    must(/\/login/.test(gp.url()), '未登入未導向登入')
    // 路人 staff 頁
    await op.goto('/admin/orgs'); await op.waitForLoadState('networkidle')
    note('一般用戶開 /admin/orgs → ' + op.url())
    const sa = await o.get('admin/orgs')
    note('一般用戶 GET /admin/orgs → ' + sa.status())
    must(sa.status() === 403, 'admin/orgs 應 403')
  }, { page: pages.outPage })

  // =========================================================== K. 手機版
  S.mOwner = await newPage({ mobile: true, user: { api: S.ownerApi } })
  await run.step('O-59', '手機 390 寬：/org、申請表（四步）、組織後台、成員、事件／據點表單、實收、編輯頁', async ({ note }) => {
    const m = S.mOwner
    const pg = [['/org', null], [`/orgs/${S.orgId}`, null], [`/orgs/${S.orgId}/members`, null], [`/orgs/${S.orgId}/events/new`, null], [`/orgs/${S.orgId}/sites/new`, null], [`/orgs/${S.orgId}/sites/${S.siteId}/receipts`, null], [`/lists/${S.siteId}/edit`, null], [`/o/${S.orgSlug}`, null], [`/e/${S.evSlug}`, null], [`/s/${S.siteSlug}`, null], ['/e', null]]
    const report = {}
    const NAMES = ['org-intro', 'console', 'members', 'event-new', 'site-new', 'receipts', 'site-edit', 'org-public', 'event-public', 'site-public', 'events-index']
    for (const [k, [u]] of pg.entries()) {
      await m.goto(u); await m.waitForLoadState('networkidle'); await sleep(300)
      const a = await mobileAudit(m)
      report[u.replace(S.orgId, '<org>').replace(S.siteId, '<site>')] = a
      await m.screenshot({ path: resolve(OUT, 'org', `O-59-m-${NAMES[k]}.png`), fullPage: true })
    }
    // 申請表手機
    const bm = await newPage({ mobile: true, user: pages.b })
    await bm.goto('/org/apply'); await bm.waitForLoadState('networkidle'); await sleep(300)
    report['/org/apply(draft)'] = await mobileAudit(bm)
    await bm.screenshot({ path: resolve(OUT, 'org', 'O-59-m-apply.png'), fullPage: true })
    // 邀請頁手機（新邀請）
    const inv = await (await S.ownerApi.post(`orgs/${S.orgId}/invites`, { data: { email: pages.out.email, role: 'volunteer' } })).json()
    const om = await newPage({ mobile: true, user: pages.out })
    await om.goto(new URL(inv.url).pathname); await om.waitForLoadState('networkidle')
    report['/org/invite/<token>'] = await mobileAudit(om)
    await S.ownerApi.delete(`orgs/${S.orgId}/invites/${inv.id}`)
    for (const [k, v] of Object.entries(report)) note(`${k}：scrollW=${v.scrollWidth}/${v.vw} over=${v.over} 小目標=${v.smallCount}${v.small.length ? ' 例 ' + v.small.slice(0, 3).join('；') : ''}${v.wide.length ? ' 超寬 ' + v.wide.join('；') : ''}`)
    S.mobile = report
    const bad = Object.entries(report).filter(([, v]) => v.over)
    must(!bad.length, '有水平捲動：' + bad.map(b => b[0]).join(', '))
    const many = Object.entries(report).filter(([, v]) => v.smallCount > 6)
    gap(!many.length, '多頁觸控目標高度 < 40px：' + many.map(([k, v]) => `${k}(${v.smallCount})`).join(', '))
  }, { page: S.mOwner })

  S.mVol = await newPage({ mobile: true, user: { api: S.volApi } })
  await run.step('O-60', '手機實際操作：志工在手機登記實收（390 寬，觸控點擊）', async ({ note }) => {
    const vp = S.mVol
    await vp.goto(`/orgs/${S.orgId}/sites/${S.siteId}/receipts`); await vp.waitForLoadState('networkidle')
    const card = vp.locator('li.c-card', { hasText: '瓶裝水' })
    await card.getByRole('button', { name: '登記實收' }).tap()
    await vp.getByRole('dialog').getByLabel('實收數量').fill('4')
    await vp.screenshot({ path: resolve(OUT, 'org', 'O-60-m-sheet.png') })
    await vp.getByRole('dialog').getByRole('button', { name: '登記', exact: true }).tap()
    await sleep(1500)
    note('手機登記後：' + (await card.innerText()).replace(/\n/g, ' / '))
    const bb = await vp.getByRole('button', { name: '登記實收' }).first().boundingBox()
    note(`登記實收鈕尺寸 ${Math.round(bb.width)}x${Math.round(bb.height)}`)
  }, { page: S.mVol })

  // =========================================================== L. 撤銷與通知
  await run.step('O-61', '【staff】撤銷整個組織（二次確認）→ 據點降為草稿、公開頁消失；組織端看到什麼', async ({ note }) => {
    const sp = pages.staff
    const card = await staffOpen(sp, S.orgId, 'verified')
    await card.getByRole('button', { name: '撤銷組織' }).click()
    await sp.getByRole('dialog').getByLabel('審核備註').fill('QA 測試：撤銷')
    await sp.getByRole('dialog').getByRole('button', { name: '撤銷組織', exact: true }).click()
    note('第二層確認文案：' + (await sp.getByRole('dialog').innerText()).replace(/\n/g, ' / ').slice(0, 120))
    await sp.getByRole('dialog').getByRole('button', { name: '撤銷組織', exact: true }).click()
    await sp.getByText('已處理').first().waitFor({ timeout: 8000 })
    note('組織狀態=' + sql(`SELECT verification_status FROM organizations WHERE id='${S.orgId}'`) + '；主據點=' + sql(`SELECT status||'/'||site_status FROM wishlists WHERE id='${S.siteId}'`))
    const g = pages.guest
    const r1 = await g.goto(`/o/${S.orgSlug}`); const r2 = await g.goto(`/s/${S.siteSlug}`); const r3 = await g.goto(`/e/${S.evSlug}`)
    note(`公開頁 HTTP：/o=${r1.status()} /s=${r2.status()} /e=${r3.status()}`)
    // 組織端
    const p = pages.owner
    await p.goto(`/orgs/${S.orgId}`); await p.waitForLoadState('networkidle')
    note('owner 開 /orgs/id：' + (await p.locator('main').innerText()).replace(/\n+/g, ' / ').slice(0, 220))
    const newSite = await p.goto(`/orgs/${S.orgId}/sites/new`); await p.waitForLoadState('networkidle')
    await p.goto('/org/apply'); await p.waitForLoadState('networkidle')
    note('owner 開 /org/apply：' + (await p.locator('main').innerText()).replace(/\n+/g, ' / ').slice(0, 200))
    const ev = await S.ownerApi.post(`orgs/${S.orgId}/events`, { data: { name: '撤銷後建事件', hazard_type: 'flood', counties: ['花蓮縣'], occurred_on: today() } })
    note(`撤銷後建事件 → ${ev.status()} ${(await ev.json().catch(() => ({}))).code ?? ''}`)
    const rc = await S.volApi.post(`orgs/${S.orgId}/receipts`, { data: { item_id: S.itemQ, qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } })
    note(`撤銷後志工登記實收 → ${rc.status()}`)
    note('撤銷後在途認捐狀態：' + sql(`SELECT status||' x'||count(*) FROM claims WHERE item_id='${S.itemQ}' GROUP BY status`).replace(/\n/g, ' | ') + '；眾籌貢獻：' + sql(`SELECT status||' x'||count(*) FROM contributions WHERE item_id='${S.itemC}' GROUP BY status`).replace(/\n/g, ' | '))
    const dm = (await listMail(pages.donor.email)).map(m => m.subject)
    note('捐助者信箱：' + JSON.stringify(dm))
    const did = (await meOf(pages.donor.api)).id
    const cid = sql(`SELECT id FROM contributions WHERE item_id='${S.itemC}' AND user_id='${did}' AND status='pledged'`)
    const bal0 = sql(`SELECT balance FROM point_wallets WHERE user_id='${did}'`)
    const wd = await pages.donor.api.delete(`contributions/${cid}`, { headers: { 'Idempotency-Key': randomUUID() } })
    note(`撤銷後捐助者自行撤回眾籌（API DELETE /contributions）→ ${wd.status()} ${(await wd.text()).slice(0, 100)}；錢包餘額 ${bal0} → ` + sql(`SELECT balance FROM point_wallets WHERE user_id='${did}'`))
    const cc = sql(`SELECT id FROM claims WHERE item_id='${S.itemQ}' AND user_id='${did}' AND status='reserved'`)
    const cx = await pages.donor.api.delete(`claims/${cc}`, { headers: { 'Idempotency-Key': randomUUID() } })
    note(`撤銷後捐助者取消認捐 → ${cx.status()}`)
    await sleep(2000)
    const rv = await waitForMail(S.ownerEmail, { subject: /驗證已撤銷/, timeout: 30000 }).catch(() => null)
    note('撤銷信：' + (rv ? `「${rv.subject}」 內文=${rv.text.replace(/\s+/g, ' ').slice(0, 240)}` : '(無)'))
    const cm = (await listMail(pages.coord.email)).map(m => m.subject)
    note('協調員信箱（被撤銷組織）：' + JSON.stringify(cm))
    gap(false, '撤銷後 /orgs/[id] 仍照常顯示並保留「新增事件／新增據點」入口（沒有「組織已被撤銷」橫幅與原因，點了才吃 ORG_NOT_VERIFIED）；原因只在 /org/apply 看得到；撤銷後志工仍可登記實收（receipts 不檢查組織狀態）；協調員與志工不會收到通知；在途認捐的捐助者沒有被通知')
  }, { page: pages.owner })

  await run.step('O-62', '通知信彙整：承辦人／協調員／志工／捐助者各收到哪些信（含補件、核准、邀請、撤銷）', async ({ note }) => {
    const subs = {}
    for (const [k, em] of Object.entries({ 承辦人: S.ownerEmail, 協調員: pages.coord.email, 志工: S.volEmail, 捐助者: pages.donor.email, 申請人B: pages.b.email })) subs[k] = (await listMail(em)).map(m => m.subject)
    for (const [k, v] of Object.entries(subs)) note(`${k}：${JSON.stringify(v)}`)
    const kinds = sql(`SELECT kind||' → '||status||' x'||count(*) FROM notifications WHERE user_id IN ('${S.ownerId}','${S.coordId}','${S.volId}') GROUP BY kind,status ORDER BY 1`)
    note('notifications 彙總（owner/coord/vol）：' + kinds.replace(/\n/g, ' | '))
    gap(subs['協調員'].length > 2, '審核結果／許可撤銷／組織撤銷信只寄給 owner；協調員與志工在這些事件上沒有任何通知，捐助者在組織被撤銷／據點被暫停時也沒有通知')
  }, { page: null })
} catch (e) {
  console.error('腳本中止：', e)
} finally {
  writeFileSync(resolve(OUT, 'org-state.json'), JSON.stringify({ ...S, ownerApi: undefined, volApi: undefined, bPage: undefined }, null, 1))
  run.save()
  await closeBrowser()
}
