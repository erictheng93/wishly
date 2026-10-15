// 三態主題：auto（跟隨系統，tokens.css 的 prefers-color-scheme）→ light → dark。
// 選擇存 localStorage 的 ws_theme 並設定 <html data-theme>；auto 則移除兩者。內嵌腳本在 nuxt.config.ts head。
const KEY = 'ws_theme'
export type ThemeMode = 'auto' | 'light' | 'dark'
const ORDER: ThemeMode[] = ['auto', 'light', 'dark']

function read(): ThemeMode {
  const t = document.documentElement.dataset.theme
  return t === 'dark' || t === 'light' ? t : 'auto'
}

// theme-color 的 media 只認系統設定；手動選擇時把兩個 meta 都改成同色，auto 時還原
const BAR = { light: '#c8102e', dark: '#0e0e0e' }
function paintBar(m: ThemeMode) {
  document.querySelectorAll<HTMLMetaElement>('meta[name="theme-color"]').forEach((el) => {
    el.content = BAR[m === 'auto' ? (el.media.includes('dark') ? 'dark' : 'light') : m]
  })
}

export function useTheme() {
  const mode = useState<ThemeMode>('theme-mode', () => 'auto')
  const sync = () => { mode.value = read(); paintBar(mode.value) }
  function toggle() {
    const next = ORDER[(ORDER.indexOf(read()) + 1) % ORDER.length]!
    const el = document.documentElement
    if (next === 'auto') delete el.dataset.theme; else el.dataset.theme = next
    try { next === 'auto' ? localStorage.removeItem(KEY) : localStorage.setItem(KEY, next) } catch { /* 無痕或封鎖儲存：只套用當次 */ }
    sync()
  }
  return { mode, sync, toggle }
}
