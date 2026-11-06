// 興趣標籤庫：須與後端 apps/api/src/interests.rs 的 INTERESTS 同序同值（tests/interests.test.ts 會比對）
export const INTEREST_GROUPS: { id: string, tags: string[] }[] = [
  { id: 'life', tags: ['travel', 'camping', 'photography', 'pets', 'gardening', 'cafe-hopping', 'reading', 'journaling'] },
  { id: 'food', tags: ['coffee', 'tea', 'baking', 'cooking', 'wine', 'dessert', 'bbq'] },
  { id: 'sports', tags: ['running', 'hiking', 'cycling', 'swimming', 'yoga', 'fitness', 'basketball'] },
  { id: 'arts', tags: ['music', 'movies', 'drawing', 'crafts', 'theater', 'anime', 'fashion'] },
  { id: 'tech', tags: ['gaming', 'gadgets', 'programming', 'board-games', 'audio'] },
  { id: 'family', tags: ['parenting', 'baby', 'home-decor', 'skincare', 'wellness', 'diy'] },
]
export const MAX_INTERESTS = 10
