// 分享頁：誰看得到捐助者名字的前端判斷（後端契約：/restricted 依觀看者顯示 friends 級名字，/public 只顯示 public 級）

/** 已登入、公開清單、名字有開、且有任何匿名項目 → 改用帶憑證路徑，好友才看得到 friends 級名字 */
export function needsCredsFetch(o: { loggedIn: boolean, visibility?: string, claimersVisible?: boolean, items?: any[] }): boolean {
  if (!o.loggedIn || o.visibility !== 'public' || !o.claimersVisible) return false
  return (o.items ?? []).some(i => i.claimers?.some((c: any) => c.anonymous) || i.contributors?.some((c: any) => c.anonymous))
}

/** 認捐 / 贊助表單的「誰看得到」說明 i18n key；可見度未載入且非匿名 / 驚喜 / 隱藏名字時回 null（不顯示） */
export function claimVisNote(o: { anon: boolean, surpriseLocked?: boolean, claimersVisible?: boolean, vis?: string }): string | null {
  if (o.anon) return 'guest.visWho.anon'
  if (o.surpriseLocked) return 'guest.visWho.surprise'
  if (o.claimersVisible === false) return 'guest.visWho.hidden'
  return o.vis === 'public' || o.vis === 'friends' || o.vis === 'private' ? `guest.visWho.${o.vis}` : null
}
