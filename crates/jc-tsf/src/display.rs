//! `ITfDisplayAttributeProvider`：組字期間的虛線底線。
//!
//! Windows 只認得「顯示屬性」這個間接層：TIP 先把一個 `GUID` 註冊成
//! `TfGuidAtom`，再透過 `ITfProperty`（`GUID_PROP_ATTRIBUTE`）把 atom 套到
//! 組字範圍；實際的顏色與底線樣式由 [`DisplayAttributeInfo`] 提供。因此這個
//! 檔案只做三件事：
//!
//! 1. 實作 [`ITfDisplayAttributeInfo`]，回傳固定的 [`input_attribute`]。
//! 2. 提供只有一項的 [`IEnumTfDisplayAttributeInfo`]。
//! 3. 在 `Activate` 時把 [`GUID_DISPLAY_ATTRIBUTE_INPUT`] 換成 atom。
//!
//! TSF 用 `QueryInterface` 從文字服務物件取得 provider，所以 provider 是
//! `TextService` 的一部分，不是獨立物件；`DllGetClassObject` 不需要多一個
//! CLSID。屬性套用失敗只記錄 log，不影響輸入。

use std::cell::Cell;

use windows::core::{implement, IUnknownImpl, Result, BSTR, GUID};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, FALSE};
use windows::Win32::Graphics::Gdi::COLOR_WINDOWTEXT;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, IEnumTfDisplayAttributeInfo, IEnumTfDisplayAttributeInfo_Impl,
    ITfCategoryMgr, ITfDisplayAttributeInfo, ITfDisplayAttributeInfo_Impl, TF_ATTR_INPUT,
    TF_CT_NONE, TF_CT_SYSCOLOR, TF_DA_COLOR, TF_DA_COLOR_0, TF_DISPLAYATTRIBUTE, TF_LS_DOT,
};

use crate::guids::GUID_DISPLAY_ATTRIBUTE_INPUT;
use crate::logging;

/// 組字用的顯示屬性：虛線底線、`TF_ATTR_INPUT` 語意。
///
/// `crText`／`crBk` 故意用 `TF_CT_NONE`：組字字串的顏色交給應用程式與 TSF
/// 決定，我們只畫底線。`bAttr = TF_ATTR_INPUT` 告訴 TSF 這是「尚未轉換的
/// 輸入」，輔助技術與部分應用程式會據此調整朗讀或外觀。
pub fn input_attribute() -> TF_DISPLAYATTRIBUTE {
    TF_DISPLAYATTRIBUTE {
        crText: TF_DA_COLOR {
            r#type: TF_CT_NONE,
            Anonymous: TF_DA_COLOR_0 { nIndex: 0 },
        },
        crBk: TF_DA_COLOR {
            r#type: TF_CT_NONE,
            Anonymous: TF_DA_COLOR_0 { nIndex: 0 },
        },
        lsStyle: TF_LS_DOT,
        fBoldLine: FALSE,
        crLine: TF_DA_COLOR {
            r#type: TF_CT_SYSCOLOR,
            Anonymous: TF_DA_COLOR_0 {
                nIndex: COLOR_WINDOWTEXT.0,
            },
        },
        bAttr: TF_ATTR_INPUT,
    }
}

/// 單一的顯示屬性實作。
///
/// 沒有可變狀態：`SetAttributeInfo` 不支援，因為顏色與樣式是編譯期常數。
#[implement(ITfDisplayAttributeInfo)]
pub struct DisplayAttributeInfo;

impl ITfDisplayAttributeInfo_Impl for DisplayAttributeInfo_Impl {
    fn GetGUID(&self) -> Result<GUID> {
        Ok(GUID_DISPLAY_ATTRIBUTE_INPUT)
    }

    fn GetDescription(&self) -> Result<BSTR> {
        Ok(BSTR::from("jc 注音：組字底線"))
    }

    fn GetAttributeInfo(&self, pda: *mut TF_DISPLAYATTRIBUTE) -> Result<()> {
        if pda.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: 呼叫端保證 pda 指向一個可寫的 TF_DISPLAYATTRIBUTE。
        unsafe {
            pda.write(input_attribute());
        }
        Ok(())
    }

    fn SetAttributeInfo(&self, _pda: *const TF_DISPLAYATTRIBUTE) -> Result<()> {
        // 顯示屬性是唯讀的；Windows 不會對 provider 呼叫這個方法。
        Err(E_NOTIMPL.into())
    }

    fn Reset(&self) -> Result<()> {
        Ok(())
    }
}

/// 只有一項的列舉器。
///
/// TSF 取得列舉器後通常只呼叫一次 `Next(1)`；`Skip`／`Reset` 仍照 COM
/// 慣例實作，讓重複列舉不會拿到兩份相同的屬性。
#[implement(IEnumTfDisplayAttributeInfo)]
pub struct DisplayAttributeEnumerator {
    /// 下一個要回傳的索引；0 代表還沒給過，1 代表已經給完。
    index: Cell<u32>,
}

impl DisplayAttributeEnumerator {
    /// 建立從頭開始的列舉器。
    pub fn new() -> Self {
        Self {
            index: Cell::new(0),
        }
    }
}

impl Default for DisplayAttributeEnumerator {
    fn default() -> Self {
        Self::new()
    }
}

impl IEnumTfDisplayAttributeInfo_Impl for DisplayAttributeEnumerator_Impl {
    fn Clone(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        let index = self.get_impl().index.get();
        let clone: IEnumTfDisplayAttributeInfo = DisplayAttributeEnumerator {
            index: Cell::new(index),
        }
        .into();
        Ok(clone)
    }

    fn Next(
        &self,
        ulcount: u32,
        rginfo: *mut Option<ITfDisplayAttributeInfo>,
        pcfetched: *mut u32,
    ) -> Result<()> {
        if pcfetched.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: 呼叫端保證 pcfetched 指向一個可寫的 u32。
        unsafe {
            *pcfetched = 0;
        }
        if ulcount == 0 || rginfo.is_null() {
            return Ok(());
        }

        let index = self.get_impl().index.get();
        if index > 0 {
            // 已經給完了；IEnum 的慣例是 fetched 留 0。
            return Ok(());
        }
        let info: ITfDisplayAttributeInfo = DisplayAttributeInfo.into();
        // SAFETY: ulcount >= 1，rginfo 是至少一個元素的緩衝。
        unsafe {
            rginfo.write(Some(info));
            *pcfetched = 1;
        }
        self.get_impl().index.set(index + 1);
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        self.get_impl().index.set(0);
        Ok(())
    }

    fn Skip(&self, ulcount: u32) -> Result<()> {
        let index = self.get_impl().index.get().saturating_add(ulcount);
        self.get_impl().index.set(index);
        Ok(())
    }
}

/// 把 [`GUID_DISPLAY_ATTRIBUTE_INPUT`] 註冊成這個 TSF 用戶端的 `TfGuidAtom`。
///
/// atom 只在同一個 TSF 用戶端內有效，不能寫進設定檔或跨行程傳遞；每次
/// `Activate` 都重新註冊一次是正常做法。失敗時回傳錯誤，由呼叫端決定是否
/// 停用底線（其餘輸入照常）。
pub fn register_guid_atom() -> Result<u32> {
    // SAFETY: Activate 期間 COM 已初始化；CategoryMgr 是系統共用的物件。
    unsafe {
        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        categories.RegisterGUID(&GUID_DISPLAY_ATTRIBUTE_INPUT)
    }
}

/// 取得 atom；失敗只回 `None` 並記錄，不阻止輸入。
pub fn try_register_guid_atom() -> Option<u32> {
    match register_guid_atom() {
        Ok(atom) => Some(atom),
        Err(error) => {
            logging::warn(format!("註冊顯示屬性 GUID 失敗，組字底線停用：{error}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_attribute_is_a_dotted_underline() {
        let attribute = input_attribute();
        assert_eq!(attribute.lsStyle, TF_LS_DOT);
        assert!(!attribute.fBoldLine.as_bool());
        assert_eq!(attribute.bAttr, TF_ATTR_INPUT);
        assert_eq!(attribute.crText.r#type, TF_CT_NONE);
        assert_eq!(attribute.crBk.r#type, TF_CT_NONE);
        assert_eq!(attribute.crLine.r#type, TF_CT_SYSCOLOR);
        // SAFETY: crLine 的 type 是 TF_CT_SYSCOLOR，讀 nIndex 是有效分支。
        assert_eq!(
            unsafe { attribute.crLine.Anonymous.nIndex },
            COLOR_WINDOWTEXT.0
        );
    }
}
