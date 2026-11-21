//! 組字區五種顯示屬性：一般細實線、使用者選字粗實線、自動修正點線、加詞標示（#123）的反白粗線，
//! 以及中英混打時按空白會組成中文的數字（#162）的灰字灰點線。
//! 前三種沿用宿主文字與背景色；標示用系統的選取色（COLOR_HIGHLIGHT／COLOR_HIGHLIGHTTEXT），
//! 讀音數字用系統的灰字色（COLOR_GRAYTEXT），適應深色與高對比主題。
use std::cell::Cell;

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
};
use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, IEnumTfDisplayAttributeInfo_Impl, ITfDisplayAttributeInfo,
    ITfDisplayAttributeInfo_Impl, TF_ATTR_CONVERTED, TF_ATTR_INPUT, TF_ATTR_TARGET_CONVERTED,
    TF_CT_COLORREF, TF_DA_COLOR, TF_DA_COLOR_0, TF_DISPLAYATTRIBUTE, TF_LS_DOT, TF_LS_SOLID,
};
use windows_core::{implement, ComObject, BSTR, GUID};

use crate::guids::{
    GUID_DISPLAY_ATTR_CLAUSE, GUID_DISPLAY_ATTR_CLAUSE_ACTIVE, GUID_DISPLAY_ATTR_CORRECTED,
    GUID_DISPLAY_ATTR_INPUT, GUID_DISPLAY_ATTR_LOCKED, GUID_DISPLAY_ATTR_MARKED,
    GUID_DISPLAY_ATTR_READING,
};

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
    pub fn guid(self) -> GUID {
        match self {
            Self::Input => GUID_DISPLAY_ATTR_INPUT,
            Self::Locked => GUID_DISPLAY_ATTR_LOCKED,
            Self::Corrected => GUID_DISPLAY_ATTR_CORRECTED,
            Self::Marked => GUID_DISPLAY_ATTR_MARKED,
            Self::Reading => GUID_DISPLAY_ATTR_READING,
            Self::Clause => GUID_DISPLAY_ATTR_CLAUSE,
            Self::ClauseActive => GUID_DISPLAY_ATTR_CLAUSE_ACTIVE,
        }
    }
    pub fn index(self) -> usize {
        self as usize
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

#[derive(Debug, PartialEq, Eq)]
pub struct StyledRange {
    pub start: i32,
    pub end: i32,
    pub kind: Kind,
}
/// TSF 位移是 UTF-16 單位，不是 Unicode scalar 或 UTF-8 bytes。
pub fn style_ranges(segments: &[crate::session::Segment]) -> Option<Vec<StyledRange>> {
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

#[implement(ITfDisplayAttributeInfo)]
pub struct InputAttribute {
    kind: Kind,
}

impl InputAttribute {
    pub fn new(kind: Kind) -> InputAttribute {
        crate::object_created();
        InputAttribute { kind }
    }
}

impl Drop for InputAttribute {
    fn drop(&mut self) {
        crate::object_destroyed();
    }
}

impl ITfDisplayAttributeInfo_Impl for InputAttribute_Impl {
    fn GetGUID(&self) -> windows_core::Result<GUID> {
        crate::guarded("GetGUID", || Ok(self.kind.guid()))
    }
    fn GetDescription(&self) -> windows_core::Result<BSTR> {
        crate::guarded("GetDescription", || Ok(BSTR::from(self.kind.description())))
    }
    fn GetAttributeInfo(&self, pda: *mut TF_DISPLAYATTRIBUTE) -> windows_core::Result<()> {
        crate::guarded("GetAttributeInfo", || {
            if pda.is_null() {
                return Err(windows::Win32::Foundation::E_POINTER.into());
            }
            let da = self.kind.attributes();
            unsafe { pda.write(da) };
            Ok(())
        })
    }
    fn SetAttributeInfo(&self, _pda: *const TF_DISPLAYATTRIBUTE) -> windows_core::Result<()> {
        crate::guarded("SetAttributeInfo", || Ok(()))
    }
    fn Reset(&self) -> windows_core::Result<()> {
        crate::guarded("Reset", || Ok(()))
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
        crate::object_created();
        AttributeEnum {
            index: Cell::new(index),
        }
    }
}

impl Drop for AttributeEnum {
    fn drop(&mut self) {
        crate::object_destroyed();
    }
}

impl IEnumTfDisplayAttributeInfo_Impl for AttributeEnum_Impl {
    fn Clone(&self) -> windows_core::Result<IEnumTfDisplayAttributeInfo> {
        crate::guarded("Clone", || {
            Ok(ComObject::new(AttributeEnum::at(self.index.get())).into_interface())
        })
    }
    fn Next(
        &self,
        count: u32,
        rginfo: *mut Option<ITfDisplayAttributeInfo>,
        fetched: *mut u32,
    ) -> windows_core::Result<()> {
        crate::guarded("Next", || {
            if (count > 0 && rginfo.is_null()) || (count != 1 && fetched.is_null()) {
                return Err(windows::Win32::Foundation::E_POINTER.into());
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
                Err(windows::Win32::Foundation::S_FALSE.into())
            }
        })
    }
    fn Reset(&self) -> windows_core::Result<()> {
        crate::guarded("Reset", || {
            self.index.set(0);
            Ok(())
        })
    }
    fn Skip(&self, count: u32) -> windows_core::Result<()> {
        crate::guarded("Skip", || {
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
                Err(windows::Win32::Foundation::S_FALSE.into())
            }
        })
    }
}
