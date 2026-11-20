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
