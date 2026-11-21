//! 組字區七種顯示屬性：一般細實線、使用者選字粗實線、自動修正點線、
//! 加詞標示（#123）的反白粗線、中英混打時按空白會組成中文的數字（#162）的灰字灰點線，
//! 以及日文変換的一般文節與目前文節（T15）。
//!
//! 移植自 0c `platforms/windows/tsf/src/displayattr.rs`（main `8f443af`）的生產碼。
//! 前三種沿用宿主文字與背景色；標示用系統的選取色（COLOR_HIGHLIGHT／COLOR_HIGHLIGHTTEXT），
//! 讀音數字用系統的灰字色（COLOR_GRAYTEXT），適應深色與高對比主題。
//!
//! T5 `edit.rs` 原本內嵌的 `Kind`／`KINDS`／`StyledRange`／`style_ranges` 已抽出到這裡。
//! [`register_atoms`] 回傳七個 TfGuidAtom，交給 `edit::apply()` 的 `attr_atoms`；
//! atom 為 0 時 `apply()` 會略過 `SetValue`。

use std::cell::Cell;

use windows::Win32::Foundation::{COLORREF, E_INVALIDARG, E_POINTER, S_FALSE};
use windows::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
};
use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, IEnumTfDisplayAttributeInfo_Impl, ITfCategoryMgr,
    ITfDisplayAttributeInfo, ITfDisplayAttributeInfo_Impl, TF_ATTR_CONVERTED, TF_ATTR_INPUT,
    TF_ATTR_TARGET_CONVERTED, TF_CT_COLORREF, TF_DA_COLOR, TF_DA_COLOR_0, TF_DISPLAYATTRIBUTE,
    TF_LS_DOT, TF_LS_SOLID,
};
use windows_core::{implement, ComObject, BSTR, GUID};

use crate::session::Segment;

/// 七種顯示屬性的 GUID。沿用 0c 已發行的值；改值等於換掉宿主認得的樣式。
mod guids {
    use windows_core::GUID;

    pub const INPUT: GUID = GUID::from_u128(0xa6ca2572_a961_4492_975f_29a4ebb7c312);
    pub const LOCKED: GUID = GUID::from_u128(0x7a4b5f38_7c42_4315_95b8_6b29f72c5b3b);
    pub const CORRECTED: GUID = GUID::from_u128(0x923cdd42_5c7e_4781_8fbe_12232f607742);
    /// 加詞標示（#123）。
    pub const MARKED: GUID = GUID::from_u128(0x7f850af6_4e7a_4b41_a24f_baeca02ce683);
    /// 中英混打時按空白會組成中文的數字（#162）。
    pub const READING: GUID = GUID::from_u128(0xcbedd798_5d14_4f4d_9917_26b934dfa003);
    /// 日文変換中的一般文節（T15）。
    pub const CLAUSE: GUID = GUID::from_u128(0x08933a6a_3bfa_4550_9423_15da82013a11);
    /// 日文変換中目前的文節（T15）。
    pub const CLAUSE_ACTIVE: GUID = GUID::from_u128(0x4f4e536f_ad3e_420b_9417_59e8bf791aa1);
}

/// 顯示屬性的種類。列舉順序就是 `attr_atoms` 的索引。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Input,
    Locked,
    Corrected,
    /// 組字中用 Shift＋←／→ 標示、要加入詞庫的範圍（核心樣式 marked，#123）。
    Marked,
    /// 中英混打時照字面顯示、按空白會組成中文的數字（核心樣式 reading，#162）。
    Reading,
    /// 日文変換中的一般文節（核心樣式 clause，T15）。
    Clause,
    /// 日文変換中目前的文節（核心樣式 clause_active，T15）。
    ClauseActive,
}

/// 七種顯示屬性的固定順序，與 `attr_atoms` 索引一致。
pub const KINDS: [Kind; 7] = [
    Kind::Input,
    Kind::Locked,
    Kind::Corrected,
    Kind::Marked,
    Kind::Reading,
    Kind::Clause,
    Kind::ClauseActive,
];

impl Kind {
    /// 這一類在 `attr_atoms` 裡的索引。
    pub fn index(self) -> usize {
        self as usize
    }

    /// 這一類註冊用的 GUID。
    pub fn guid(self) -> GUID {
        match self {
            Self::Input => guids::INPUT,
            Self::Locked => guids::LOCKED,
            Self::Corrected => guids::CORRECTED,
            Self::Marked => guids::MARKED,
            Self::Reading => guids::READING,
            Self::Clause => guids::CLAUSE,
            Self::ClauseActive => guids::CLAUSE_ACTIVE,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Input => "靈犀注音：組字",
            Self::Locked => "靈犀注音：已選字",
            Self::Corrected => "靈犀注音：自動修正",
            Self::Marked => "靈犀注音：加入詞庫的範圍",
            Self::Reading => "靈犀注音：按空白會變成中文的數字",
            Self::Clause => "靈犀：日文文節",
            Self::ClauseActive => "靈犀：目前的日文文節",
        }
    }

    fn attributes(self) -> TF_DISPLAYATTRIBUTE {
        let color = |index| TF_DA_COLOR {
            r#type: TF_CT_COLORREF,
            Anonymous: TF_DA_COLOR_0 {
                cr: COLORREF(unsafe { GetSysColor(index) }),
            },
        };
        if self == Self::Reading {
            // 比照常見注音輸入法：數字畫成灰字、灰色點線底線，看得出還沒定案（讀音 ㄉㄢ 由候選窗顯示）。
            // 會畫文字色的宿主（記事本）是灰字；只畫底線的宿主（WPF、Chromium）至少是灰色點線，
            // 和自動修正的黑色點線分得開。
            return TF_DISPLAYATTRIBUTE {
                crText: color(COLOR_GRAYTEXT),
                crLine: color(COLOR_GRAYTEXT),
                lsStyle: TF_LS_DOT,
                bAttr: TF_ATTR_INPUT,
                ..Default::default()
            };
        }
        if self == Self::Marked {
            // 反白（系統選取色）加上選取色的粗底線：會畫背景的宿主（記事本、Word）反白，只畫底線的宿主
            // （WPF、Chromium）也看得出和已選字（黑色粗線）不同。顏色在查詢時才讀系統設定，跟著主題與高對比；
            // 用 COLORREF 而不是 TF_CT_SYSCOLOR：實測 Edge 與 WPF 對 TF_CT_SYSCOLOR 什麼都不畫，
            // 換成 COLORREF 後底線才變成選取色（兩者都不畫背景色）。底線用點線：WPF、Chromium 只看底線，
            // 實心粗線跟平常組字的底線只差粗細，點線一眼分得出來（2026-10-18 使用者在 A/B 截圖中選定；
            // 記事本、Word 的反白蓋過底線，看起來不變）。自動修正也是點線，但那是黑色細線。
            return TF_DISPLAYATTRIBUTE {
                crText: color(COLOR_HIGHLIGHTTEXT),
                crBk: color(COLOR_HIGHLIGHT),
                crLine: color(COLOR_HIGHLIGHT),
                lsStyle: TF_LS_DOT,
                fBoldLine: true.into(),
                bAttr: TF_ATTR_TARGET_CONVERTED,
                ..Default::default()
            };
        }
        if self == Self::Clause || self == Self::ClauseActive {
            // 日文変換：一般文節細實線、目前文節粗實線。bAttr 用 TSF 的已変換／目前変換，
            // 支援原生顯示的宿主（Word 等）也會畫出自己的文節樣式；只畫底線的宿主仍分得出來。
            let active = self == Self::ClauseActive;
            return TF_DISPLAYATTRIBUTE {
                lsStyle: TF_LS_SOLID,
                fBoldLine: active.into(),
                bAttr: if active {
                    TF_ATTR_TARGET_CONVERTED
                } else {
                    TF_ATTR_CONVERTED
                },
                ..Default::default()
            };
        }
        TF_DISPLAYATTRIBUTE {
            lsStyle: if self == Self::Corrected {
                TF_LS_DOT
            } else {
                TF_LS_SOLID
            },
            fBoldLine: (self == Self::Locked).into(),
            bAttr: if self == Self::Locked {
                TF_ATTR_CONVERTED
            } else {
                TF_ATTR_INPUT
            },
            ..Default::default()
        }
    }
}

/// 一段要套用顯示屬性的 UTF-16 位移範圍。
#[derive(Debug, PartialEq, Eq)]
pub struct StyledRange {
    pub start: i32,
    pub end: i32,
    pub kind: Kind,
}

/// 把 snapshot 的 fragment 轉成 UTF-16 位移範圍。TSF 位移是 UTF-16 單位，不是 scalar 或 bytes。
pub fn style_ranges(segments: &[Segment]) -> Option<Vec<StyledRange>> {
    let mut ranges = Vec::new();
    let mut start = 0i32;
    for segment in segments {
        let length = i32::try_from(segment.text.encode_utf16().count()).ok()?;
        let end = start.checked_add(length)?;
        if length > 0 {
            let kind = match segment.style.as_str() {
                "locked" => Kind::Locked,
                "corrected" => Kind::Corrected,
                "marked" => Kind::Marked,
                "reading" => Kind::Reading,
                "clause" => Kind::Clause,
                "clause_active" => Kind::ClauseActive,
                _ => Kind::Input,
            };
            ranges.push(StyledRange { start, end, kind });
        }
        start = end;
    }
    Some(ranges)
}

/// 向 category manager 註冊七種 GUID，回傳 `edit::apply()` 要的 `attr_atoms`。
/// `category` 為 None 或單筆註冊失敗時，那一格是 0，`apply()` 會略過對應的 `SetValue`。
/// 0c 在 `service.rs` 的 Activate 用 `msctf::create_category_mgr()` 取得 manager 後呼叫同樣的邏輯。
pub fn register_atoms(category: Option<&ITfCategoryMgr>) -> [u32; KINDS.len()] {
    KINDS.map(|kind| {
        category
            .and_then(|mgr| unsafe { mgr.RegisterGUID(&kind.guid()) }.ok())
            .unwrap_or(0)
    })
}

/// 建立單一顯示屬性介面，供 `ITfDisplayAttributeProvider::GetDisplayAttributeInfo` 回傳。
pub fn attribute_info(kind: Kind) -> ITfDisplayAttributeInfo {
    ComObject::new(InputAttribute::new(kind)).into_interface()
}

/// 依 GUID 找出對應的顯示屬性；找不到回 `E_INVALIDARG`。
pub fn info_for_guid(guid: &GUID) -> windows_core::Result<ITfDisplayAttributeInfo> {
    match KINDS.into_iter().find(|kind| kind.guid() == *guid) {
        Some(kind) => Ok(attribute_info(kind)),
        None => Err(E_INVALIDARG.into()),
    }
}

/// 建立七種顯示屬性的列舉器，供 `ITfDisplayAttributeProvider::EnumDisplayAttributeInfo` 回傳。
pub fn enumerator() -> IEnumTfDisplayAttributeInfo {
    ComObject::new(AttributeEnum::new()).into_interface()
}

/// COM 邊界不得 unwind；與 0c `lib.rs::guarded` 行為相同。
/// adapter spike 不帶檔案 I/O，panic 時只回 `E_FAIL`。
fn guarded<T, F: FnOnce() -> windows_core::Result<T>>(what: &str, f: F) -> windows_core::Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            let _ = what;
            Err(windows::Win32::Foundation::E_FAIL.into())
        }
    }
}

/// COM 物件計數。新 crate 還沒有 DLL 生命週期管理，T6 接到 `lib.rs` 的 LIVE_OBJECTS。
fn object_created() {}
fn object_destroyed() {}

#[implement(ITfDisplayAttributeInfo)]
pub struct InputAttribute {
    kind: Kind,
}

impl InputAttribute {
    pub fn new(kind: Kind) -> InputAttribute {
        object_created();
        InputAttribute { kind }
    }
}

impl Drop for InputAttribute {
    fn drop(&mut self) {
        object_destroyed();
    }
}

impl ITfDisplayAttributeInfo_Impl for InputAttribute_Impl {
    fn GetGUID(&self) -> windows_core::Result<GUID> {
        guarded("GetGUID", || Ok(self.kind.guid()))
    }
    fn GetDescription(&self) -> windows_core::Result<BSTR> {
        guarded("GetDescription", || Ok(BSTR::from(self.kind.description())))
    }
    fn GetAttributeInfo(&self, pda: *mut TF_DISPLAYATTRIBUTE) -> windows_core::Result<()> {
        guarded("GetAttributeInfo", || {
            if pda.is_null() {
                return Err(E_POINTER.into());
            }
            let da = self.kind.attributes();
            unsafe { pda.write(da) };
            Ok(())
        })
    }
    fn SetAttributeInfo(&self, _pda: *const TF_DISPLAYATTRIBUTE) -> windows_core::Result<()> {
        guarded("SetAttributeInfo", || Ok(()))
    }
    fn Reset(&self) -> windows_core::Result<()> {
        guarded("Reset", || Ok(()))
    }
}

#[implement(IEnumTfDisplayAttributeInfo)]
pub struct AttributeEnum {
    index: Cell<u32>,
}

impl AttributeEnum {
    pub fn new() -> AttributeEnum {
        AttributeEnum::at(0)
    }

    fn at(index: u32) -> AttributeEnum {
        object_created();
        AttributeEnum {
            index: Cell::new(index),
        }
    }
}

impl Default for AttributeEnum {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AttributeEnum {
    fn drop(&mut self) {
        object_destroyed();
    }
}

impl IEnumTfDisplayAttributeInfo_Impl for AttributeEnum_Impl {
    fn Clone(&self) -> windows_core::Result<IEnumTfDisplayAttributeInfo> {
        guarded("Clone", || {
            Ok(ComObject::new(AttributeEnum::at(self.index.get())).into_interface())
        })
    }
    fn Next(
        &self,
        count: u32,
        rginfo: *mut Option<ITfDisplayAttributeInfo>,
        fetched: *mut u32,
    ) -> windows_core::Result<()> {
        guarded("Next", || {
            if (count > 0 && rginfo.is_null()) || (count != 1 && fetched.is_null()) {
                return Err(E_POINTER.into());
            }
            let mut n = 0u32;
            while n < count && (self.index.get() as usize) < KINDS.len() {
                let kind = KINDS[self.index.get() as usize];
                let info: ITfDisplayAttributeInfo =
                    ComObject::new(InputAttribute::new(kind)).into_interface();
                // [out] 陣列在呼叫端通常沒有初始化（#89）：只能 write，不能用 `*p = …`，
                // 後者會先把垃圾值當成舊的介面 Release，宿主（Outlook）當場崩潰。
                unsafe { rginfo.add(n as usize).write(Some(info)) };
                self.index.set(self.index.get() + 1);
                n += 1;
            }
            if !fetched.is_null() {
                unsafe { fetched.write(n) };
            }
            if n == count {
                Ok(())
            } else {
                Err(S_FALSE.into())
            }
        })
    }
    fn Reset(&self) -> windows_core::Result<()> {
        guarded("Reset", || {
            self.index.set(0);
            Ok(())
        })
    }
    fn Skip(&self, count: u32) -> windows_core::Result<()> {
        guarded("Skip", || {
            let remaining = KINDS.len() as u32 - self.index.get();
            self.index.set(
                self.index
                    .get()
                    .saturating_add(count)
                    .min(KINDS.len() as u32),
            );
            if count <= remaining {
                Ok(())
            } else {
                Err(S_FALSE.into())
            }
        })
    }
}
