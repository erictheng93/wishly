//! 興趣標籤庫：id 與前端 utils/interests.ts 相同順序（apps/web/tests/interests.test.ts 會比對）。

pub const INTERESTS: &[&str] = &[
    "travel",
    "camping",
    "photography",
    "pets",
    "gardening",
    "cafe-hopping",
    "reading",
    "journaling",
    "coffee",
    "tea",
    "baking",
    "cooking",
    "wine",
    "dessert",
    "bbq",
    "running",
    "hiking",
    "cycling",
    "swimming",
    "yoga",
    "fitness",
    "basketball",
    "music",
    "movies",
    "drawing",
    "crafts",
    "theater",
    "anime",
    "fashion",
    "gaming",
    "gadgets",
    "programming",
    "board-games",
    "audio",
    "parenting",
    "baby",
    "home-decor",
    "skincare",
    "wellness",
    "diy",
];

/// 全部在庫內（長度上限由呼叫端檢查）
pub fn valid(ids: &[String]) -> bool {
    ids.iter().all(|i| INTERESTS.contains(&i.as_str()))
}
