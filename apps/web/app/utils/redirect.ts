// 登入後導回：只接受站內相對路徑，擋 //host 與 /\host（瀏覽器會當成同協定外站）的 open redirect
export const safeRedirect = (raw: unknown, fallback = '/dashboard') => {
  const s = String(raw || '')
  return s.startsWith('/') && !s.startsWith('//') && !s.startsWith('/\\') ? s : fallback
}
/** 前往登入頁並在登入後回到 path（保留 query，例如 ?x=1） */
export const loginUrl = (path: string) => ({ path: '/login', query: { redirect: safeRedirect(path, '/') } })
