//! 候選窗的純版面計算：不碰 Win32、Direct2D，Linux 也能跑單元測試。
//!
//! 版面一律用 DIP（96 DPI 的邏輯像素）表示；像素換算、監視器工作區夾取、滑鼠命中
//! 都在這裡做成純函式，Windows 端的 `ui_window` 只負責量文字、畫圖與送視窗訊息。
//!
//! 一個候選是一列（多欄時同一列排多個），列高固定；讀音提示在最上面，頁碼在最下面
//! 靠右。提示列與頁碼不參與命中：點在空白處不會選字。

/// DIP 換算的基準 DPI。
pub const BASE_DPI: u32 = 96;

/// 視窗內緣留白（DIP）。
pub const PAD: f32 = 8.0;
/// 每一格內文字與格線的留白（DIP）。
pub const INNER_PAD: f32 = 6.0;
/// 一列候選的高度（DIP）。
pub const ROW_HEIGHT: f32 = 26.0;
/// 列與列之間的空隙（DIP）。
pub const ROW_GAP: f32 = 2.0;
/// 讀音提示列的高度（DIP）。
pub const HEADER_HEIGHT: f32 = 20.0;
/// 頁碼列的高度（DIP）。
pub const FOOTER_HEIGHT: f32 = 16.0;
/// 視窗最小寬度（DIP）：只有一個窄候選時不要縮成一條。
pub const MIN_WIDTH: f32 = 96.0;
/// 視窗最大寬度（DIP）：長候選不要橫跨整個螢幕。
pub const MAX_WIDTH: f32 = 520.0;
/// 多欄之間的間隙（DIP）。
pub const COLUMN_GAP: f32 = 4.0;
/// 編號鍵與候選文字之間的距離（DIP）。
pub const KEY_GAP: f32 = 6.0;
/// 候選文字與附註之間的距離（DIP）。
pub const NOTE_GAP: f32 = 8.0;
/// 圓角半徑（DIP）。
pub const CORNER_RADIUS: f32 = 5.0;
/// 欄數上限；核心目前不送欄數（0），一律單欄。
pub const MAX_COLUMNS: usize = 10;

/// 候選文字的字級（DIP）。
pub const TEXT_SIZE: f32 = 15.0;
/// 編號鍵的字級（DIP）。
pub const KEY_SIZE: f32 = 11.0;
/// 讀音提示、附註、頁碼的字級（DIP）。
pub const SMALL_SIZE: f32 = 12.0;

/// 量文字時要用的字級；實際字型由 Windows 端的 DirectWrite 決定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    /// 候選文字。
    Candidate,
    /// 編號鍵。
    Key,
    /// 讀音提示、附註、頁碼。
    Small,
}

impl TextStyle {
    /// 這個樣式在 DIP 下的字級，供量測端建立對應的 `IDWriteTextFormat`。
    pub fn size(self) -> f32 {
        match self {
            TextStyle::Candidate => TEXT_SIZE,
            TextStyle::Key => KEY_SIZE,
            TextStyle::Small => SMALL_SIZE,
        }
    }
}

/// 以 DIP 表示的矩形；原點是候選窗客戶區左上角。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DipRect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl DipRect {
    pub fn new(left: f32, top: f32, width: f32, height: f32) -> DipRect {
        DipRect {
            left,
            top,
            width,
            height,
        }
    }

    pub fn right(&self) -> f32 {
        self.left + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.top + self.height
    }

    /// 點是否落在矩形內；右邊界與下邊界不算（與 Win32 的 `PtInRect` 相反，方便相鄰格子不重疊）。
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right() && y >= self.top && y < self.bottom()
    }
}

/// 螢幕像素座標的矩形（`GetTextExt`、`GetMonitorInfo` 的單位）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> i32 {
        (self.right - self.left).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.bottom - self.top).max(0)
    }
}

/// 一個候選：`id` 回送給引擎，`key` 為 `None` 或空字串時不畫編號。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidate {
    pub id: i64,
    pub text: String,
    pub note: String,
    pub key: Option<String>,
}

/// 一次呈現要畫的內容。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Content {
    pub candidates: Vec<Candidate>,
    /// 反白候選在 `candidates` 的索引；超出範圍或負數視為沒有反白。
    pub highlight: i64,
    /// 1 起算的頁碼與總頁數；`pages <= 1` 不畫頁碼。
    pub page: i64,
    pub pages: i64,
    /// 讀音提示，畫在候選上方。
    pub reading: String,
    /// 要求的欄數；0 與 1 都當單欄。
    pub columns: usize,
}

/// 編號鍵的文字與位置。
#[derive(Clone, Debug, PartialEq)]
pub struct KeyLabel {
    pub text: String,
    pub rect: DipRect,
}

/// 一列候選的版面：格線、編號、文字、附註各自的位置。
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: i64,
    /// 整格範圍（含留白），滑鼠命中用這個。
    pub rect: DipRect,
    pub key: Option<KeyLabel>,
    pub text: String,
    pub text_rect: DipRect,
    pub note: String,
    pub note_rect: Option<DipRect>,
}

/// 一行文字與位置（讀音提示、頁碼）。
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub rect: DipRect,
}

/// 算好的版面；`width`／`height` 是視窗的 DIP 尺寸。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub header: Vec<Line>,
    pub rows: Vec<Row>,
    pub pager: Option<Line>,
    /// 反白的是 `rows` 的第幾項；沒有反白時 `None`。
    pub highlighted: Option<usize>,
}

impl Layout {
    /// 整格的圓角矩形，畫反白底色用。
    pub fn row_highlight(&self, index: usize) -> Option<DipRect> {
        self.rows.get(index).map(|row| row.rect)
    }
}

/// 頁碼文字：`pages > 1` 才畫，頁碼超出範圍時夾在合法區間。
pub fn pager_text(page: i64, pages: i64) -> Option<String> {
    (pages > 1).then(|| format!("{} / {}", page.clamp(1, pages), pages))
}

/// DIP 換成像素；`dpi` 為 0 時當 96，避免除以零。
pub fn px_from_dip(dip: f32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { BASE_DPI } else { dpi };
    (dip * dpi as f32 / BASE_DPI as f32).round() as i32
}

/// 像素換成 DIP（滑鼠座標用）。
pub fn dip_from_px(px: i32, dpi: u32) -> f32 {
    let dpi = if dpi == 0 { BASE_DPI } else { dpi };
    px as f32 * BASE_DPI as f32 / dpi as f32
}

/// 視窗像素尺寸，夾進工作區；工作區比 1 小也至少留 1 像素。
pub fn fit_size(size: (i32, i32), work: Rect) -> (i32, i32) {
    (
        size.0.clamp(1, work.width().max(1)),
        size.1.clamp(1, work.height().max(1)),
    )
}

/// 候選窗位置：預設貼在插入點下方，下方放不下就翻到上方，最後夾進工作區。
pub fn place(caret: Rect, size: (i32, i32), work: Rect, gap: i32) -> (i32, i32) {
    let (width, height) = fit_size(size, work);
    let below = caret.bottom + gap;
    let above = caret.top.saturating_sub(gap).saturating_sub(height);
    let y = if below + height <= work.bottom {
        below
    } else if above >= work.top {
        above
    } else {
        below
    };
    let max_x = (work.right - width).max(work.left);
    let max_y = (work.bottom - height).max(work.top);
    (caret.left.clamp(work.left, max_x), y.clamp(work.top, max_y))
}

/// 滑鼠命中的候選 ID；`x`／`y` 是 DIP 的客戶區座標。
pub fn hit_test(layout: &Layout, x: f32, y: f32) -> Option<i64> {
    layout
        .rows
        .iter()
        .find(|row| row.rect.contains(x, y))
        .map(|row| row.id)
}

/// 算版面。`measure` 回傳文字在指定樣式下的寬度（DIP），Windows 端由 DirectWrite 提供。
pub fn layout(content: &Content, measure: impl Fn(&str, TextStyle) -> f32) -> Layout {
    let columns = content.columns.clamp(1, MAX_COLUMNS);
    let count = content.candidates.len();
    let mut rows = Vec::with_capacity(count);
    let mut cells = Vec::with_capacity(count);
    let mut widest = 0.0f32;

    for candidate in &content.candidates {
        let key = candidate
            .key
            .as_deref()
            .filter(|key| !key.is_empty())
            .map(|key| (key.to_owned(), measure(key, TextStyle::Key)));
        let text_width = measure(&candidate.text, TextStyle::Candidate);
        let note = (!candidate.note.is_empty()).then(|| {
            (
                candidate.note.clone(),
                measure(&candidate.note, TextStyle::Small),
            )
        });
        let mut natural = INNER_PAD * 2.0 + text_width;
        if let Some((_, key_width)) = &key {
            natural += key_width + KEY_GAP;
        }
        if let Some((_, note_width)) = &note {
            natural += NOTE_GAP + note_width;
        }
        widest = widest.max(natural);
        cells.push((text_width, key, note, natural));
    }

    let content_width = if columns == 1 {
        widest
    } else {
        let column_width = widest.max(1.0);
        column_width * columns as f32 + COLUMN_GAP * (columns as f32 - 1.0)
    };
    let reading_width = if content.reading.is_empty() {
        0.0
    } else {
        measure(&content.reading, TextStyle::Small)
    };
    let pager = pager_text(content.page, content.pages);
    let pager_width = pager
        .as_deref()
        .map(|text| measure(text, TextStyle::Small))
        .unwrap_or(0.0);
    let width =
        (content_width.max(reading_width).max(pager_width) + PAD * 2.0).clamp(MIN_WIDTH, MAX_WIDTH);
    let inner = width - PAD * 2.0;
    let cell_width = if columns == 1 {
        inner
    } else {
        ((inner - COLUMN_GAP * (columns as f32 - 1.0)) / columns as f32).max(1.0)
    };

    let header = if content.reading.is_empty() {
        Vec::new()
    } else {
        vec![Line {
            text: content.reading.clone(),
            rect: DipRect::new(PAD, PAD, inner, HEADER_HEIGHT),
        }]
    };
    let header_height = if header.is_empty() {
        0.0
    } else {
        HEADER_HEIGHT
    };
    let row_count = count.div_ceil(columns);
    let rows_height = if row_count == 0 {
        0.0
    } else {
        row_count as f32 * ROW_HEIGHT + (row_count as f32 - 1.0) * ROW_GAP
    };
    let footer_height = if pager.is_some() { FOOTER_HEIGHT } else { 0.0 };
    let height = PAD * 2.0 + header_height + rows_height + footer_height;

    for (index, (candidate, (text_width, key, note, _))) in
        content.candidates.iter().zip(cells).enumerate()
    {
        let column = index % columns;
        let line = index / columns;
        let cell_left = PAD + column as f32 * (cell_width + COLUMN_GAP);
        let cell_top = PAD + header_height + line as f32 * (ROW_HEIGHT + ROW_GAP);
        let cell = DipRect::new(cell_left, cell_top, cell_width, ROW_HEIGHT);
        let mut text_left = cell_left + INNER_PAD;
        let key = key.map(|(text, key_width)| {
            let rect = DipRect::new(text_left, cell_top, key_width, ROW_HEIGHT);
            text_left += key_width + KEY_GAP;
            KeyLabel { text, rect }
        });
        let text_rect = DipRect::new(text_left, cell_top, text_width, ROW_HEIGHT);
        let note_rect = note.map(|(_, note_width)| {
            DipRect::new(
                text_left + text_width + NOTE_GAP,
                cell_top,
                note_width,
                ROW_HEIGHT,
            )
        });
        rows.push(Row {
            id: candidate.id,
            rect: cell,
            key,
            text: candidate.text.clone(),
            text_rect,
            note: candidate.note.clone(),
            note_rect,
        });
    }

    let pager = pager.map(|text| Line {
        text,
        rect: DipRect::new(PAD, height - PAD - FOOTER_HEIGHT, inner, FOOTER_HEIGHT),
    });
    let highlighted = usize::try_from(content.highlight)
        .ok()
        .filter(|index| *index < rows.len());

    Layout {
        width,
        height,
        header,
        rows,
        pager,
        highlighted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定字寬的量測：每字半個字級寬，測版面時不需要真的字型。
    fn measure(text: &str, style: TextStyle) -> f32 {
        text.chars().count() as f32 * style.size() * 0.5
    }

    fn candidate(id: i64, text: &str, key: Option<&str>, note: &str) -> Candidate {
        Candidate {
            id,
            text: text.to_owned(),
            note: note.to_owned(),
            key: key.map(str::to_owned),
        }
    }

    fn content(candidates: Vec<Candidate>) -> Content {
        Content {
            candidates,
            highlight: 0,
            page: 1,
            pages: 1,
            reading: String::new(),
            columns: 0,
        }
    }

    #[test]
    fn single_column_stacks_candidates_with_fixed_row_height() {
        let content = content(vec![
            candidate(1, "價", Some("1"), ""),
            candidate(2, "架", Some("2"), "音 ㄐㄧㄚˋ"),
            candidate(3, "假", Some("3"), ""),
        ]);
        let layout = layout(&content, measure);
        assert_eq!(layout.rows.len(), 3);
        assert_eq!(layout.header.len(), 0);
        assert_eq!(layout.pager, None);
        assert_eq!(layout.width, MIN_WIDTH, "窄候選不縮成小於最小寬度");
        let expected_height = PAD * 2.0 + 3.0 * ROW_HEIGHT + 2.0 * ROW_GAP;
        assert!((layout.height - expected_height).abs() < f32::EPSILON);
        for (index, row) in layout.rows.iter().enumerate() {
            assert_eq!(row.rect.left, PAD);
            assert_eq!(
                row.rect.top,
                PAD + index as f32 * (ROW_HEIGHT + ROW_GAP),
                "第 {index} 列的位置"
            );
            assert_eq!(row.rect.width, MIN_WIDTH - PAD * 2.0);
            assert_eq!(row.rect.height, ROW_HEIGHT);
            // 編號在文字左邊，附註在文字右邊。
            let key = row.key.as_ref().expect("編號鍵");
            assert_eq!(key.text, (index + 1).to_string());
            assert!(key.rect.right() <= row.text_rect.left);
            if let Some(note) = &row.note_rect {
                assert!(note.left >= row.text_rect.right());
            }
        }
        assert_eq!(hit_test(&layout, PAD + 1.0, PAD + 1.0), Some(1));
        assert_eq!(
            hit_test(&layout, PAD + 1.0, PAD + ROW_HEIGHT + ROW_GAP + 1.0),
            Some(2)
        );
        // 列間空隙與視窗內緣不選字。
        assert_eq!(
            hit_test(&layout, PAD + 1.0, PAD + ROW_HEIGHT + ROW_GAP * 0.5),
            None
        );
        assert_eq!(hit_test(&layout, PAD * 0.5, PAD + 1.0), None);
        assert_eq!(hit_test(&layout, layout.width + 1.0, PAD + 1.0), None);
    }

    #[test]
    fn empty_key_is_not_drawn_and_text_starts_at_the_cell_edge() {
        let content = content(vec![
            candidate(7, "我", Some(""), ""),
            candidate(8, "窩", None, ""),
        ]);
        let layout = layout(&content, measure);
        for row in &layout.rows {
            assert!(row.key.is_none(), "空字串與 None 都不畫編號");
            assert_eq!(row.text_rect.left, row.rect.left + INNER_PAD);
        }
        assert_eq!(hit_test(&layout, PAD + INNER_PAD, PAD + INNER_PAD), Some(7));
    }

    #[test]
    fn highlight_is_clamped_to_existing_rows() {
        let mut content = content(vec![
            candidate(1, "甲", None, ""),
            candidate(2, "乙", None, ""),
        ]);
        content.highlight = 1;
        let built = layout(&content, measure);
        assert_eq!(built.highlighted, Some(1));
        assert_eq!(built.row_highlight(1), Some(built.rows[1].rect));
        assert_eq!(built.row_highlight(2), None);

        for highlight in [-1, 2, 99] {
            content.highlight = highlight;
            assert_eq!(layout(&content, measure).highlighted, None, "{highlight}");
        }
    }

    #[test]
    fn columns_wrap_rows_left_to_right_without_overlap() {
        let mut content = content(vec![
            candidate(1, "一", Some("1"), ""),
            candidate(2, "二", Some("2"), ""),
            candidate(3, "三", Some("3"), ""),
            candidate(4, "四", Some("4"), ""),
            candidate(5, "五", Some("5"), ""),
        ]);
        content.columns = 3;
        let layout = layout(&content, measure);
        assert_eq!(layout.rows.len(), 5);
        assert_eq!(layout.rows[0].rect.top, layout.rows[1].rect.top);
        assert_eq!(layout.rows[1].rect.top, layout.rows[2].rect.top);
        assert_eq!(
            layout.rows[3].rect.top,
            PAD + ROW_HEIGHT + ROW_GAP,
            "第 4 項換到第二列"
        );
        assert_eq!(layout.rows[0].rect.left, PAD);
        assert!(layout.rows[1].rect.left >= layout.rows[0].rect.right());
        assert!(
            layout.rows[1].rect.left <= layout.rows[0].rect.right() + COLUMN_GAP + f32::EPSILON
        );
        // 每格等寬，整排不超出視窗。
        let width = layout.rows[0].rect.width;
        for row in &layout.rows {
            assert!((row.rect.width - width).abs() < f32::EPSILON);
        }
        assert!(layout.rows[2].rect.right() <= layout.width - PAD + f32::EPSILON);
        // 命中第 5 項（第二列第二格）。
        let fifth = &layout.rows[4].rect;
        assert_eq!(
            hit_test(&layout, fifth.left + 1.0, fifth.top + 1.0),
            Some(5)
        );
    }

    #[test]
    fn reading_hint_takes_a_header_row_and_is_not_clickable() {
        let mut content = content(vec![candidate(1, "價", Some("1"), "")]);
        content.reading = "ㄐㄧㄚˋ".into();
        let layout = layout(&content, measure);
        assert_eq!(layout.header.len(), 1);
        assert_eq!(layout.header[0].text, "ㄐㄧㄚˋ");
        assert_eq!(layout.header[0].rect.top, PAD);
        assert_eq!(layout.rows[0].rect.top, PAD + HEADER_HEIGHT);
        assert_eq!(
            layout.height,
            PAD * 2.0 + HEADER_HEIGHT + ROW_HEIGHT,
            "讀音列加進高度"
        );
        assert_eq!(
            hit_test(&layout, PAD + 1.0, PAD + 1.0),
            None,
            "讀音列不選字"
        );
        assert_eq!(
            hit_test(&layout, PAD + 1.0, PAD + HEADER_HEIGHT + 1.0),
            Some(1)
        );
    }

    #[test]
    fn pager_only_appears_when_there_is_more_than_one_page() {
        let mut content = content(vec![candidate(1, "價", Some("1"), "")]);
        content.pages = 1;
        assert_eq!(pager_text(1, 1), None);
        assert_eq!(pager_text(1, 0), None, "jc 目前沒有頁數");
        assert_eq!(layout(&content, measure).pager, None);

        content.page = 2;
        content.pages = 5;
        let layout = layout(&content, measure);
        assert_eq!(pager_text(2, 5).as_deref(), Some("2 / 5"));
        let pager = layout.pager.as_ref().expect("多頁時要畫頁碼");
        assert_eq!(pager.text, "2 / 5");
        assert_eq!(pager.rect.bottom(), layout.height - PAD);
        assert!(pager.rect.right() <= layout.width - PAD + f32::EPSILON);
        assert_eq!(
            layout.height,
            PAD * 2.0 + ROW_HEIGHT + FOOTER_HEIGHT,
            "頁碼列加進高度"
        );
        // 頁碼不是候選，點了不會選字。
        assert_eq!(
            hit_test(&layout, pager.rect.left + 1.0, pager.rect.top + 1.0),
            None
        );
        // 頁碼超出範圍時夾回合法值。
        assert_eq!(pager_text(9, 5).as_deref(), Some("5 / 5"));
        assert_eq!(pager_text(0, 5).as_deref(), Some("1 / 5"));
    }

    #[test]
    fn dpi_conversion_rounds_to_nearest_pixel() {
        assert_eq!(px_from_dip(15.0, 96), 15);
        assert_eq!(px_from_dip(15.0, 144), 23);
        assert_eq!(px_from_dip(15.0, 120), 19);
        assert_eq!(px_from_dip(15.0, 0), 15, "DPI 0 當 96");
        assert_eq!(dip_from_px(24, 144), 16.0);
        assert_eq!(dip_from_px(19, 120), 15.2);
        // 來回換算不漂移超過一個像素。
        for dpi in [96, 120, 144, 168, 192, 240, 288, 384] {
            let px = px_from_dip(ROW_HEIGHT, dpi);
            assert!(
                (dip_from_px(px, dpi) - ROW_HEIGHT).abs() <= 0.5,
                "dpi {dpi}"
            );
        }
    }

    #[test]
    fn window_opens_below_the_caret_and_flips_above_when_it_does_not_fit() {
        let work = Rect::new(0, 0, 1920, 1080);
        let size = (200, 120);
        // 上方空間充足：貼在插入點下方。
        let caret = Rect::new(300, 200, 302, 220);
        assert_eq!(place(caret, size, work, 4), (300, 224));
        // 下方放不下：翻到插入點上方。
        let caret = Rect::new(300, 1000, 302, 1020);
        assert_eq!(place(caret, size, work, 4), (300, 1000 - 4 - 120));
        // 上下都放不下：仍然顯示在下方，再夾進工作區。
        let caret = Rect::new(300, 0, 302, 1080);
        let (_, y) = place(caret, size, work, 4);
        assert_eq!(y, 1080 - 120);
    }

    #[test]
    fn window_is_clamped_inside_the_monitor_work_area() {
        // 次要監視器：工作區原點不是 (0,0)。
        let work = Rect::new(1920, -200, 3200, 880);
        let size = (300, 100);
        // 插入點在最右邊：視窗往左夾。
        let caret = Rect::new(3190, 100, 3192, 120);
        assert_eq!(place(caret, size, work, 4).0, 3200 - 300);
        // 插入點在工作區左邊界外：夾回左邊界。
        let caret = Rect::new(1500, 100, 1502, 120);
        assert_eq!(place(caret, size, work, 4).0, 1920);
        // 工作區比視窗小：尺寸先夾小，位置不超出工作區。
        let tiny = Rect::new(0, 0, 120, 80);
        let (x, y) = place(Rect::new(10, 10, 12, 30), (400, 300), tiny, 4);
        assert_eq!((x, y), (0, 0));
        assert_eq!(fit_size((400, 300), tiny), (120, 80));
    }

    #[test]
    fn zero_width_work_area_does_not_panic() {
        let work = Rect::new(0, 0, 0, 0);
        assert_eq!(fit_size((100, 100), work), (1, 1));
        let (x, y) = place(Rect::new(50, 50, 51, 60), (100, 100), work, 4);
        assert_eq!((x, y), (0, 0));
    }
}
