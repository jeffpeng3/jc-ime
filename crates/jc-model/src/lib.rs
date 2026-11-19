//! ONNX Runtime 接線（`ort` crate）。模型維持 ONNX，不進 git。
//!
//! 骨架：先定義模型角色與可觀測的載入狀態，實際 session 在 M4 接上。

/// 模型角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 注音選字模型。
    ZhSelect,
    /// 注音標點模型。
    ZhPunct,
    /// 日文候選重排模型。
    JaRerank,
    /// 日文標點模型。
    JaPunct,
}

/// 模型可用狀態；缺模型時引擎退回統計路徑，不應失敗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Loaded { role: Role, bytes: u64 },
    Unavailable { role: Role, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_is_not_an_error() {
        let a = Availability::Unavailable {
            role: Role::JaRerank,
            reason: "no model".into(),
        };
        matches!(a, Availability::Unavailable { .. });
    }
}
