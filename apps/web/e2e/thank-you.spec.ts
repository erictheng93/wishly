import { expect, test } from '@playwright/test'
import { createList, loginBrowser, registerUser, sql } from './helpers'

test('匿名捐助者在驚喜解鎖後收到一張私人謝卡', async ({ page, browser }) => {
  const owner = await registerUser({ tag: 'owner' })
  const donor = await registerUser({ tag: 'donor' })
  const list = await createList(owner, { surprise: true })
  const claim = await donor.api.post(`items/${list.items[0]!.id}/claims`, {
    data: { qty: 1, is_anonymous: true }, headers: { 'Idempotency-Key': crypto.randomUUID() },
  })
  expect(claim.ok()).toBe(true)
  const { claim: { id } } = await claim.json()
  for (const status of ['purchased', 'delivered']) {
    expect((await donor.api.patch(`claims/${id}`, { data: { status } })).ok()).toBe(true)
  }

  await loginBrowser(page.context(), owner)
  await page.goto(`/lists/${list.id}/progress`)
  await expect(page.getByText(/名單會在活動日當天解鎖/)).toBeVisible()
  const compose = page.getByRole('button', { name: '寫謝卡' })
  await expect(compose).toHaveCount(0)
  sql(`UPDATE wishlists SET event_date = CURRENT_DATE - 1 WHERE id = '${list.id}'`)
  await page.reload()
  await expect(page.getByText(/匿名朋友/)).toBeVisible()
  await expect(page.getByText(donor.name)).toHaveCount(0)
  await compose.click()
  const dialog = page.getByRole('dialog', { name: '寫謝卡' })
  const send = dialog.getByRole('button', { name: '傳送謝卡' })
  await expect(send).toBeDisabled()
  const message = '謝謝你送的奶瓶，寶寶已經用上了！'
  await dialog.getByRole('textbox').fill(message)
  await send.click()
  await expect(page.getByText('謝卡已傳送')).toBeVisible()
  await page.reload()
  await expect(page.getByText('謝卡已傳送')).toBeVisible()
  await expect(compose).toHaveCount(0)

  // The notification must still open the card when the gift is beyond two pages of history.
  sql(`INSERT INTO claims (item_id, user_id, claimer_name, qty, status)
       SELECT item_id, user_id, claimer_name, 1, 'cancelled'
       FROM claims CROSS JOIN generate_series(1, 101) WHERE id = '${id}'`)
  const donorPage = await browser.newPage()
  try {
    await loginBrowser(donorPage.context(), donor)
    await donorPage.goto('/me/notifications?category=gifts')
    await donorPage.locator('.c-inbox').getByRole('button', { name: /謝卡/ }).click()
    await expect(donorPage).toHaveURL(new RegExp(`/me/claims\\?claim=${id}$`))
    await expect(donorPage.getByText(message, { exact: true })).toBeVisible()
    await donorPage.getByRole('link', { name: '查看所有送禮紀錄' }).click()
    const more = donorPage.getByRole('button', { name: '載入更多' })
    await expect(more).toBeVisible()
    await expect(donorPage.getByText(message)).toHaveCount(0)
    await more.click()
    await expect(donorPage.locator('summary')).toContainText('100')
    await more.click()
    await expect(donorPage.getByText(message, { exact: true })).toBeVisible()
  } finally {
    await donorPage.close()
  }
})
