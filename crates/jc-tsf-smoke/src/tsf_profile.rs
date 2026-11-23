//! 用 TSF API 查目前使用者的 profile 狀態（檢查 10）。
//!
//! 只讀機碼不夠：`LanguageProfile` 存在不代表使用者已啟用，也不代表
//! `EnumLanguageProfiles` 會列出它。這裡直接問 msctf：
//!
//! - `ITfInputProcessorProfiles::IsEnabledLanguageProfile`
//! - `ITfInputProcessorProfileMgr::GetActiveProfile`
//! - `ITfInputProcessorProfiles::EnumLanguageProfiles(0x0404)`

use windows::core::Interface;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr, ITfInputProcessorProfiles,
    GUID_TFCAT_TIP_KEYBOARD, TF_INPUTPROCESSORPROFILE, TF_IPPMF_FORPROCESS, TF_LANGUAGEPROFILE,
    TF_PROFILETYPE_INPUTPROCESSOR,
};

use crate::error::SmokeError;
use crate::product;
use crate::profile::{self, UserProfileVerdict};
use crate::report;

/// TSF API 回報的使用者 profile 狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserProfileState {
    /// `IsEnabledLanguageProfile` 的結果；`Err` 是 HRESULT 字串。
    pub enabled: Result<bool, String>,
    /// `EnumLanguageProfiles(0x0404)` 是否列出本產品。
    pub enumerated: Result<bool, String>,
    /// 列舉到的 profile 總數。
    pub enumerated_count: usize,
    /// `GetActiveProfile` 回報的 profile GUID；沒有作用中的 profile 時為 `None`。
    pub active: Result<Option<String>, String>,
    /// 目前作用中的 profile 是不是本產品。
    pub active_is_ours: bool,
}

impl UserProfileState {
    /// 由兩個必要結果得到判定；API 失敗時回 `None`，由呼叫端當 `[SKIP]`。
    pub fn verdict(&self) -> Option<UserProfileVerdict> {
        Some(profile::user_profile_verdict(
            *self.enabled.as_ref().ok()?,
            *self.enumerated.as_ref().ok()?,
        ))
    }

    /// 一行摘要，供人類輸出與 JSON 的 `detail` 共用。
    pub fn detail(&self) -> String {
        let enabled = match &self.enabled {
            Ok(value) => format!("IsEnabledLanguageProfile={value}"),
            Err(error) => format!("IsEnabledLanguageProfile 失敗（{error}）"),
        };
        let enumerated = match &self.enumerated {
            Ok(value) => format!(
                "EnumLanguageProfiles(0x0404)={value}（共 {} 個 profile）",
                self.enumerated_count
            ),
            Err(error) => format!("EnumLanguageProfiles(0x0404) 失敗（{error}）"),
        };
        let active = match &self.active {
            Ok(Some(guid)) if self.active_is_ours => format!("目前啟用 {guid}（本產品）"),
            Ok(Some(guid)) => format!("目前啟用 {guid}（不是本產品）"),
            Ok(None) => "目前沒有作用中的鍵盤 profile".to_string(),
            Err(error) => format!("GetActiveProfile 失敗（{error}）"),
        };
        format!("{enabled}；{enumerated}；{active}")
    }
}

/// 建立 `ITfInputProcessorProfiles` 並查三件事。
///
/// 建立 COM 物件失敗才算整項失敗；單一 API 失敗會留在欄位裡，讓報表顯示細節。
pub fn query() -> Result<UserProfileState, SmokeError> {
    // SAFETY: 呼叫端已 CoInitializeEx；這是標準的 in-proc COM 建立。
    let profiles: ITfInputProcessorProfiles = unsafe {
        CoCreateInstance(
            &CLSID_TF_InputProcessorProfiles,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
    }
    .map_err(|error| {
        SmokeError::hresult(
            "CoCreateInstance(CLSID_TF_InputProcessorProfiles)",
            error.code(),
        )
    })?;

    // SAFETY: profiles 是有效的 TSF 介面；GUID 都是常數。
    let enabled = unsafe {
        profiles.IsEnabledLanguageProfile(
            &product::CLSID_JC_TSF,
            product::LANGID_ZH_TW,
            &product::GUID_PROFILE_JC_TSF,
        )
    }
    .map(|value| value.as_bool())
    .map_err(|error| report::hex(error.code().0 as u32));

    let (enumerated, enumerated_count) = match enum_language_profiles(&profiles) {
        Ok((found, count)) => (Ok(found), count),
        Err(error) => (Err(error), 0),
    };

    let (active, active_is_ours) = match active_profile(&profiles) {
        Ok(Some(guid)) => {
            let is_ours = guid == product::GUID_PROFILE_JC_TSF;
            (Ok(Some(product::registry_key(&guid))), is_ours)
        }
        Ok(None) => (Ok(None), false),
        Err(error) => (Err(error), false),
    };

    Ok(UserProfileState {
        enabled,
        enumerated,
        enumerated_count,
        active,
        active_is_ours,
    })
}

/// 在目前執行緒把本產品設為作用中的輸入法。
///
/// 註冊 profile 加上 `EnableLanguageProfile` 只代表「已為這個使用者安裝並啟用」，
/// 不等於這個執行緒正在用它。msctf 要等到 `ActivateProfile` 才會載入 TIP，並用
/// 真正的 TIP client id 呼叫 `ITfTextInputProcessor::Activate`。少了這一步，
/// `ITfClientId::GetClientId` 建出來的 id 背後沒有任何 TIP，
/// `AdviseKeyEventSink` 就回 `E_INVALIDARG`（run 45140346900 的 `tsf.log` 只有
/// `activate 開始 tid=29` 一行，之後什麼都沒有）。
///
/// `TF_IPPMF_FORPROCESS` 讓影響只留在本行程：不改系統或使用者的預設輸入法，
/// 行程結束就沒了。這正是宿主程式（記事本等）切到本產品時走的路。
pub fn activate_ours() -> Result<(), SmokeError> {
    // SAFETY: 呼叫端已 CoInitializeEx；這是標準的 in-proc COM 建立。
    let profiles: ITfInputProcessorProfiles = unsafe {
        CoCreateInstance(
            &CLSID_TF_InputProcessorProfiles,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
    }
    .map_err(|error| {
        SmokeError::hresult(
            "CoCreateInstance(CLSID_TF_InputProcessorProfiles)",
            error.code(),
        )
    })?;
    let manager: ITfInputProcessorProfileMgr = profiles.cast().map_err(|error| {
        SmokeError::hresult("ITfInputProcessorProfiles::cast(ProfileMgr)", error.code())
    })?;
    // SAFETY: manager 有效；GUID 都是常數，hkl 傳預設值代表不指定鍵盤配置。
    unsafe {
        manager.ActivateProfile(
            TF_PROFILETYPE_INPUTPROCESSOR,
            product::LANGID_ZH_TW,
            &product::CLSID_JC_TSF,
            &product::GUID_PROFILE_JC_TSF,
            HKL::default(),
            TF_IPPMF_FORPROCESS,
        )
    }
    .map_err(|error| SmokeError::hresult("ActivateProfile", error.code()))
}

/// 列舉 `0x0404` 的 profile，回傳（是否含本產品, 總數）。
fn enum_language_profiles(profiles: &ITfInputProcessorProfiles) -> Result<(bool, usize), String> {
    // SAFETY: profiles 有效；回傳的列舉器由本函式持有。
    let enumerator = unsafe { profiles.EnumLanguageProfiles(product::LANGID_ZH_TW) }
        .map_err(|error| report::hex(error.code().0 as u32))?;

    let mut found = false;
    let mut count = 0usize;
    loop {
        let mut item = [TF_LANGUAGEPROFILE::default(); 1];
        let mut fetched = 0u32;
        // SAFETY: item 是一個元素的緩衝，fetched 由 API 寫入。
        match unsafe { enumerator.Next(&mut item, &mut fetched) } {
            Ok(()) => {}
            Err(error) => return Err(report::hex(error.code().0 as u32)),
        }
        if fetched == 0 {
            break;
        }
        count += 1;
        if item[0].clsid == product::CLSID_JC_TSF
            && item[0].guidProfile == product::GUID_PROFILE_JC_TSF
        {
            found = true;
        }
    }
    Ok((found, count))
}

/// 問 `GetActiveProfile` 目前作用中的鍵盤 profile。
fn active_profile(
    profiles: &ITfInputProcessorProfiles,
) -> Result<Option<windows::core::GUID>, String> {
    let manager: ITfInputProcessorProfileMgr = profiles
        .cast()
        .map_err(|error| report::hex(error.code().0 as u32))?;
    let mut profile = TF_INPUTPROCESSORPROFILE::default();
    // SAFETY: manager 有效；profile 是本機的可寫輸出。
    match unsafe { manager.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut profile) } {
        Ok(()) => Ok(Some(profile.guidProfile)),
        // 沒有作用中的 profile 是正常情況（例如目前是英文鍵盤）。
        Err(error) => Err(report::hex(error.code().0 as u32)),
    }
}
