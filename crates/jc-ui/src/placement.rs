//! 螢幕座標可為負數；候選窗在縮放後也必須留在工作區。移植自 0c。
use windows::Win32::Foundation::RECT;

/// 把候選窗放在游標下方；放不下改上方；最後 clamp 在螢幕工作區內。
pub fn place(x: i32, y: i32, caret_height: i32, width: i32, height: i32, work: RECT) -> RECT {
    let width = width
        .max(1)
        .min(work.right.saturating_sub(work.left).max(1));
    let height = height
        .max(1)
        .min(work.bottom.saturating_sub(work.top).max(1));
    let left = x.clamp(work.left, work.right.saturating_sub(width).max(work.left));
    let below = y.saturating_add(caret_height).saturating_add(2);
    let top = if below.saturating_add(height) <= work.bottom {
        below
    } else {
        y.saturating_sub(height).saturating_sub(2)
    }
    .clamp(work.top, work.bottom.saturating_sub(height).max(work.top));
    RECT {
        left,
        top,
        right: left.saturating_add(width),
        bottom: top.saturating_add(height),
    }
}
