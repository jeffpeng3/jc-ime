//! 離線的表情與顏文字選單。
//!
//! 這裡只有手選的 Unicode 字串，沒有關鍵字詞典、註解或語言模型關係。
//! 常用＋最近排序沿用前身 `core/emoji/frecency.go`：每個項目記衰減後的
//! 選取次數，常用的穩定排前面，很久沒用的慢慢退後；新選的會往前，但一次
//! 不會直接超過天天用的。分數要明顯超過前一名才交換，兩個差不多的表情不會
//! 每次互換。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 「最近」清單的上限。
pub const MAX_RECENT: usize = 64;

/// 等號的保留 ID：重播時不會把字面等號與表情混淆。
pub const EQUALS_ID: usize = 1 << 20;

/// 表情符號。每一項都是手選的 Unicode 字串，保留所有序列字元。
const FACES: &[&str] = &[
    "😀",
    "😃",
    "😄",
    "😁",
    "😆",
    "😅",
    "😂",
    "🤣",
    "😊",
    "🙂",
    "🙃",
    "😉",
    "😍",
    "🥰",
    "😘",
    "😋",
    "😎",
    "🤩",
    "🥳",
    "🤔",
    "🤗",
    "🫡",
    "🤭",
    "🫢",
    "😐",
    "😑",
    "😶",
    "🙄",
    "😏",
    "😬",
    "😮",
    "😯",
    "😲",
    "😳",
    "🥺",
    "🥹",
    "😢",
    "😭",
    "😤",
    "😠",
    "😡",
    "🤯",
    "😱",
    "😰",
    "😥",
    "😓",
    "😴",
    "🥱",
    "😷",
    "🤒",
    "👍",
    "👎",
    "👌",
    "✌️",
    "🤞",
    "🤟",
    "🤘",
    "🤙",
    "👏",
    "🙌",
    "🙏",
    "👋",
    "💪",
    "🤝",
    "🫶",
    "👍🏻",
    "👍🏼",
    "👍🏽",
    "👍🏾",
    "👍🏿",
    "❤️",
    "🧡",
    "💛",
    "💚",
    "💙",
    "💜",
    "🖤",
    "🤍",
    "💕",
    "💔",
    "💯",
    "✅",
    "❌",
    "❓",
    "❗",
    "🔥",
    "✨",
    "🎉",
    "🎊",
    "🎁",
    "☀️",
    "🌈",
    "🌧️",
    "⭐",
    "🌙",
    "🌸",
    "🍀",
    "🐱",
    "🐶",
    "🐼",
    "☕",
    "🍵",
    "🍺",
    "🍰",
    "🍕",
    "🍜",
    "🚀",
    "🎮",
    "💻",
    "📱",
    "👨‍👩‍👧‍👦",
    "👩‍💻",
    "👨‍💻",
    "🇹🇼",
    "🇯🇵",
    "🇺🇸",
    "1️⃣",
    "2️⃣",
    "3️⃣",
    "🏳️‍🌈",
];

/// 顏文字；與表情共用同一套最近排序。
const KAOMOJI: &[&str] = &[
    "(^_^)",
    "(≧▽≦)",
    "(＾▽＾)",
    "(・∀・)",
    "(⌒▽⌒)",
    "(o^▽^o)",
    "(T_T)",
    "(;_;)",
    "(╥﹏╥)",
    "(>_<)",
    "(；ω；)",
    "(ಥ_ಥ)",
    "(・_・)",
    "(・・?)",
    "(°_°)",
    "(O_O)",
    "(￣▽￣)",
    "(￣ー￣)",
    "(－_－)",
    "(¬_¬)",
    "(｀Д´)",
    "(╯°□°)╯︵ ┻━┻",
    "┬─┬ノ(・_・ノ)",
    "¯\\_(ツ)_/¯",
    "(づ｡◕‿‿◕｡)づ",
    "(｡♥‿♥｡)",
    "(♡˙︶˙♡)",
    "(=^･ω･^=)",
    "(•ᴥ•)",
    "(•̀ᴗ•́)و",
];

/// 回報 `text` 是不是表情或顏文字。
pub fn contains(text: &str) -> bool {
    FACES.contains(&text) || KAOMOJI.contains(&text)
}

/// 穩定的候選 ID，不隨最近順序改變。
///
/// 字面等號固定回傳 [`EQUALS_ID`]；不在目錄裡時回傳 `None`。
pub fn id(text: &str) -> Option<usize> {
    if text == "=" {
        return Some(EQUALS_ID);
    }
    if let Some(index) = FACES.iter().position(|item| *item == text) {
        return Some(index);
    }
    KAOMOJI
        .iter()
        .position(|item| *item == text)
        .map(|index| FACES.len() + index)
}

/// 丟掉不認識的值與重複項目，不排序、不保存任意使用者文字。
///
/// 回傳值不與目錄或輸入共用。
pub fn clean_recent(recent: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for text in recent {
        if contains(text) && !out.iter().any(|item| item == text) {
            out.push(text.clone());
            if out.len() == MAX_RECENT {
                break;
            }
        }
    }
    out
}

/// 表情選單內容：最近使用且屬於這一類的項目排前面，其餘照目錄順序。
pub fn palette(kind: &str, recent: &[String]) -> Vec<String> {
    let base = if kind == "kaomoji" { KAOMOJI } else { FACES };
    let mut out = Vec::new();
    for text in clean_recent(recent) {
        if base.contains(&text.as_str()) {
            out.push(text);
        }
    }
    for text in base {
        if !out.iter().any(|item| item == text) {
            out.push((*text).to_string());
        }
    }
    out
}

/// 一個項目的常用分數：`at` 時的衰減後次數。
///
/// `at` 為 0 表示時間未知（由舊版的最近清單轉來），下一次有時鐘的選取時才
/// 定錨，在那之前不衰減。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    /// 衰減後的選取次數。
    #[serde(rename = "s")]
    pub score: f64,
    /// 分數定錨時間（秒）；0 表示未知。
    #[serde(rename = "t", default, skip_serializing_if = "is_zero")]
    pub at: i64,
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

/// 兩週沒用分數減半（秒）。
pub const HALF_LIFE: i64 = 14 * 86400;

/// 分數要超過前一名的這個倍數才交換。
pub const SWAP_MARGIN: f64 = 1.5;

/// 回傳 `now` 時的分數。
pub fn decayed(stat: Stat, now: i64) -> f64 {
    if stat.at == 0 || now <= stat.at {
        return stat.score;
    }
    stat.score * f64::exp2(-((now - stat.at) as f64) / HALF_LIFE as f64)
}

/// 把舊版的最近清單轉成初始分數，順序不變：第一個略低於 2、最後一個略高於 1，
/// 新選一次（1 分）不會立刻超過任何一個。
pub fn seed_stats(recent: &[String]) -> HashMap<String, Stat> {
    let mut stats = HashMap::new();
    let n = recent.len() as f64;
    for (index, text) in recent.iter().enumerate() {
        stats.insert(
            text.clone(),
            Stat {
                score: 1.0 + (n - index as f64) / (n + 1.0),
                at: 0,
            },
        );
    }
    stats
}

/// 記下一次選取，回傳新的順序與分數（都不與輸入共用）。
///
/// `order` 是目前保存的順序。
pub fn pick(
    order: &[String],
    stats: &HashMap<String, Stat>,
    text: &str,
    now: i64,
) -> (Vec<String>, HashMap<String, Stat>) {
    rank(&clean_recent(order), stats, text, now, MAX_RECENT)
}

/// [`pick`] 不綁目錄的部分：`order` 須已去重、只含可接受的項目，
/// 清單最多 `limit` 項。符號選單的「最近」分頁也用它。
///
/// 回傳值都不與輸入共用。
pub fn rank(
    order: &[String],
    stats: &HashMap<String, Stat>,
    text: &str,
    now: i64,
    limit: usize,
) -> (Vec<String>, HashMap<String, Stat>) {
    let mut order = order.to_vec();
    let mut next = HashMap::with_capacity(stats.len() + 1);
    for (key, stat) in stats {
        let mut stat = *stat;
        if stat.at == 0 && now > 0 {
            stat.at = now; // 定錨：從現在開始衰減
        }
        next.insert(key.clone(), stat);
    }
    let current = next
        .get(text)
        .copied()
        .unwrap_or(Stat { score: 0.0, at: 0 });
    next.insert(
        text.to_string(),
        Stat {
            score: decayed(current, now) + 1.0,
            at: now,
        },
    );
    if !order.iter().any(|item| item == text) {
        if order.len() >= limit {
            // 滿了：讓出分數最低的位置，新選的一定進得來。
            let mut weakest = 0;
            for index in 0..order.len() {
                let score = |key: &String| {
                    decayed(
                        next.get(key).copied().unwrap_or(Stat { score: 0.0, at: 0 }),
                        now,
                    )
                };
                if score(&order[index]) <= score(&order[weakest]) {
                    weakest = index;
                }
            }
            order.remove(weakest);
        }
        // 新項目從清單尾端（其餘目錄之前）開始往前爬。
        order.push(text.to_string());
    }
    let score = |key: &String| {
        decayed(
            next.get(key).copied().unwrap_or(Stat { score: 0.0, at: 0 }),
            now,
        )
    };
    for i in 1..order.len() {
        let mut j = i;
        while j > 0 && score(&order[j]) > SWAP_MARGIN * score(&order[j - 1]) {
            order.swap(j, j - 1);
            j -= 1;
        }
    }
    order.truncate(limit);
    // 分數只留給清單裡的項目，大小有界。
    next.retain(|key, _| order.iter().any(|item| item == key));
    (order, next)
}

/// 只保留目錄裡、而且在順序清單中的項目；分數不合理的丟掉。
pub fn clean_stats(stats: &HashMap<String, Stat>, order: &[String]) -> HashMap<String, Stat> {
    keep_stats(stats, &clean_recent(order))
}

/// 只保留在順序清單 `order`（已清理過）中的項目；分數不合理的丟掉。
///
/// 沒有任何項目時回傳空 map，存檔來回一致。
pub fn keep_stats(stats: &HashMap<String, Stat>, order: &[String]) -> HashMap<String, Stat> {
    let mut out = HashMap::new();
    for (key, stat) in stats {
        if order.iter().any(|item| item == key) && stat.score > 0.0 && stat.score.is_finite() {
            out.insert(key.clone(), *stat);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86400;

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn test_seed_keeps_order_and_one_pick_does_not_jump() {
        let recent = owned(&["😂", "👍", "🙏", "🔥"]);
        let stats = seed_stats(&recent);
        let (order, stats) = pick(&recent, &stats, "🎉", 100 * DAY);
        assert_eq!(
            order,
            owned(&["😂", "👍", "🙏", "🔥", "🎉"]),
            "升級後不洗牌、新選一次不超過舊的"
        );
        // 連續選幾次就往前爬，但要明顯超過才交換。
        let mut order = order;
        let mut stats = stats;
        for i in 0..3 {
            (order, stats) = pick(&order, &stats, "🎉", 100 * DAY + i);
        }
        assert_eq!(order[0], "🎉", "常用的排前面：{order:?}");
    }

    #[test]
    fn test_hysteresis_keeps_similar_items_stable() {
        let mut order = Vec::new();
        let mut stats = HashMap::new();
        let now = 10 * DAY;
        for _ in 0..5 {
            (order, stats) = pick(&order, &stats, "😀", now);
            (order, stats) = pick(&order, &stats, "😃", now);
        }
        let before = order.clone();
        (order, _) = pick(&order, &stats, "😃", now); // 只多一次：差距不到 1.5 倍
        assert_eq!(order, before, "差不多的兩個不互換");
    }

    #[test]
    fn test_decay_lets_old_favourites_fall_back() {
        let mut order = Vec::new();
        let mut stats = HashMap::new();
        for _ in 0..10 {
            (order, stats) = pick(&order, &stats, "🐱", DAY);
        }
        let later = DAY + 8 * HALF_LIFE;
        (order, stats) = pick(&order, &stats, "🐶", later);
        (order, stats) = pick(&order, &stats, "🐶", later);
        assert_eq!(order[0], "🐶", "很久沒用的退後：{order:?} {stats:?}");
    }

    #[test]
    fn test_pick_bounds_and_clean() {
        let mut order = Vec::new();
        let mut stats = HashMap::new();
        for i in 0..MAX_RECENT + 10 {
            (order, stats) = pick(&order, &stats, FACES[i], DAY + i as i64);
        }
        assert_eq!(order.len(), MAX_RECENT);
        assert_eq!(stats.len(), MAX_RECENT);
        let mut input = HashMap::new();
        input.insert("not-emoji".to_string(), Stat { score: 1.0, at: 0 });
        input.insert(FACES[0].to_string(), Stat { score: 2.0, at: 0 });
        input.insert(FACES[1].to_string(), Stat { score: -1.0, at: 0 });
        let clean = clean_stats(&input, &owned(&[FACES[0], FACES[1]]));
        assert_eq!(clean.len(), 1, "{clean:?}");
    }

    #[test]
    fn test_palette_recent_first_and_stable_ids() {
        let recent = owned(&["🙏", "任意文字", "😂", "😂"]);
        let items = palette("emoji", &recent);
        assert_eq!(items[0], "🙏");
        assert_eq!(items[1], "😂");
        assert_eq!(items.iter().filter(|item| *item == "😂").count(), 1);
        assert_eq!(id("😂"), Some(6));
        assert_eq!(id("="), Some(EQUALS_ID));
        assert_eq!(id("任意文字"), None);
        let kaomoji = palette("kaomoji", &owned(&["😂", "(^_^)"]));
        assert_eq!(kaomoji, owned(KAOMOJI));
        assert_eq!(clean_recent(&recent), owned(&["🙏", "😂"]));
    }

    /// 資料檔是手動整理的顏文字表；缺檔時 skip，存在時驗證格式與首列。
    #[test]
    fn test_kaomoji_data_rows_are_well_formed() {
        let path = jc_testdata::require_data!("ja-src/symbols/kaomoji.tsv");
        let text = std::fs::read_to_string(&path).expect("讀取 kaomoji.tsv");
        let rows: Vec<&str> = text
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        assert!(!rows.is_empty(), "kaomoji.tsv 沒有資料列");
        for row in &rows {
            assert_eq!(row.split('\t').count(), 4, "欄位數：{row:?}");
        }
        assert_eq!(
            rows[0].split('\t').collect::<Vec<_>>(),
            ["かおもじ", "(^^)", "笑顔の顔文字", "顔文字"]
        );
    }
}
