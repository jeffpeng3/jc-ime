//! 模型角色與 manifest 解析。

use std::fmt;
use std::str::FromStr;

/// 模型角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// 注音選字模型。
    ZhSelect,
    /// 日文候選重排模型。
    JaRerank,
}

impl Role {
    /// manifest 使用的角色名稱。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ZhSelect => "zh-select",
            Self::JaRerank => "ja-rerank",
        }
    }

    /// 所有角色，固定順序。
    pub const fn all() -> [Role; 2] {
        [Self::ZhSelect, Self::JaRerank]
    }

    /// `drop-punct` 移除、但舊資料目錄的 manifest 仍可能列出的角色。
    ///
    /// 解析集合 manifest 時遇到這些名稱要直接跳過，不印警告；它們不會回到
    /// [`Role`] 列舉，也不該讓每次啟動都洗版。
    pub fn is_legacy_role(name: &str) -> bool {
        matches!(name, "zh-punct" | "ja-punct")
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "zh-select" => Ok(Self::ZhSelect),
            "ja-rerank" => Ok(Self::JaRerank),
            other => Err(format!("未知的模型角色：{other}")),
        }
    }
}
