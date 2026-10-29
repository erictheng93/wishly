// 生日清單通知（D65）：何時不能開啟。回傳停用原因，null 表示可開。順序同後端檢查。
export function birthdayNotifyBlock(u: { birthday?: string | null, birthday_visibility?: string } | null | undefined, visibility: string): 'required' | 'private' | 'listPrivate' | null {
  if (!u?.birthday) return 'required'
  if (u.birthday_visibility === 'private') return 'private'
  if (visibility === 'private') return 'listPrivate'
  return null
}
