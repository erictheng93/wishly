// 預設封面的分享預覽圖：LINE/FB 不吃 SVG，改 public/covers/*.svg 後執行 `node scripts/cover-og.mjs` 重產 1200×630 PNG。
// 做法：把 120×120 的 viewBox 左右撐到 1200:630，底色與斜紋 rect 一起拉寬（假設 SVG 維持現有的底色/斜紋 rect 寫法）。
// shortcut: sharp 是 Nuxt 的間接依賴，沒列進 package.json；哪天被移除就改成 devDependency。
import { readdirSync, readFileSync } from 'node:fs'
import sharp from 'sharp'

const dir = new URL('../public/covers/', import.meta.url)
const w = 120 * 1200 / 630, x = (120 - w) / 2
for (const f of readdirSync(dir).filter(f => f.endsWith('.svg'))) {
  const svg = readFileSync(new URL(f, dir), 'utf8')
    .replace('viewBox="0 0 120 120"', `viewBox="${x} 0 ${w} 120" width="1200" height="630"`)
    .replace('<rect width="120" height="120"', `<rect x="${x}" width="${w}" height="120"`)
    .replace('<rect y="108" width="120"', `<rect x="${x}" y="108" width="${w}"`)
  await sharp(Buffer.from(svg)).png({ compressionLevel: 9, palette: true }).toFile(new URL(f.replace('.svg', '.png'), dir).pathname)
  console.log(f.replace('.svg', '.png'))
}
