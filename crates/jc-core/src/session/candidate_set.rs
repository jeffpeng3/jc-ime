//! 候選清單的共用組裝器。
//!
//! 注音、日文與混打三個 lane 的 rebuild 都只提供三樣東西：
//!
//! 1. 原始 hypotheses：lane 已用 nbest／個人學習／rerank 排好的 [`Candidate`]。
//! 2. lane 專屬 extras：注音模糊音、英文拼字建議等。
//! 3. 顯示字。
//!
//! 排序完之後的「插入 extras → 去重 → 截斷 → 顯示字排第一 → 產生候選與
//! [`CandidateAction`] 的平行清單」都在這裡做一次。排序本身留在 lane 的
//! rank hook：三種 hypotheses 的型別與學習資料不同，但交給 builder 時都已經
//! 是 [`Candidate`] 的順序，builder 不再重排，名額與順序因此與改動前相同。

use crate::candidate::Candidate;

use super::CandidateAction;

/// 字面去重時要比對的形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SurfaceMatch {
    /// 完全相同的字串（注音、日文）。
    Exact,
    /// ASCII 大小寫不敏感（混打的英文候選）。
    AsciiLower,
}

impl SurfaceMatch {
    fn key(self, surface: &str) -> String {
        match self {
            Self::Exact => surface.to_string(),
            Self::AsciiLower => surface.to_lowercase(),
        }
    }
}

/// 候選清單的組裝設定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AssemblyOptions {
    /// 最後是否再整份去重；只保留每個字面的第一項。
    pub(super) dedupe: bool,
    /// 顯示字是否要移到第一項。混打的一般候選維持原本順序，不啟用。
    pub(super) display_first: bool,
}

impl Default for AssemblyOptions {
    fn default() -> Self {
        Self {
            dedupe: false,
            display_first: true,
        }
    }
}

/// 組裝完成的候選與平行動作。
pub(super) struct AssembledCandidates {
    /// 候選，順序即顯示順序。
    pub(super) candidates: Vec<Candidate>,
    /// 與 [`Self::candidates`] 平行的套用動作；`None` 代表沿用既有路徑。
    pub(super) actions: Vec<Option<CandidateAction>>,
}

impl AssembledCandidates {
    /// 只要候選清單時使用。
    pub(super) fn into_candidates(self) -> Vec<Candidate> {
        self.candidates
    }
}

struct CandidateEntry {
    candidate: Candidate,
    action: Option<CandidateAction>,
}

/// 一份還在組裝中的候選清單。
pub(super) struct CandidateSet {
    display: String,
    limit: usize,
    matching: SurfaceMatch,
    seen: Vec<String>,
    entries: Vec<CandidateEntry>,
}

impl CandidateSet {
    /// 建立空清單；`display` 是最後要排到第一的顯示字。
    pub(super) fn new(display: impl Into<String>, limit: usize, matching: SurfaceMatch) -> Self {
        Self {
            display: display.into(),
            limit,
            matching,
            seen: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// 以 lane 已排好序的 hypotheses 建立清單。
    pub(super) fn from_hypotheses(
        display: impl Into<String>,
        limit: usize,
        matching: SurfaceMatch,
        hypotheses: impl IntoIterator<Item = Candidate>,
    ) -> Self {
        let mut set = Self::new(display, limit, matching);
        for candidate in hypotheses {
            set.push(candidate);
        }
        set
    }

    /// 第一項的字面；空清單回傳 `None`。
    pub(super) fn first_surface(&self) -> Option<&str> {
        self.entries
            .first()
            .map(|entry| entry.candidate.surface.as_str())
    }

    /// 這個字面是否已在清單裡。
    pub(super) fn contains(&self, surface: &str) -> bool {
        let key = self.matching.key(surface);
        self.seen.contains(&key)
    }

    /// 先記住字面但不加入候選；混打要用原始按鍵佔住去重鍵。
    pub(super) fn reserve(&mut self, surface: &str) {
        let key = self.matching.key(surface);
        if !self.seen.contains(&key) {
            self.seen.push(key);
        }
    }

    /// 直接加入一項，不檢查重複。
    pub(super) fn push(&mut self, candidate: Candidate) {
        self.seen.push(self.matching.key(&candidate.surface));
        self.entries.push(CandidateEntry {
            candidate,
            action: None,
        });
    }

    /// 加入一項並附上平行動作。
    pub(super) fn push_with_action(
        &mut self,
        candidate: Candidate,
        action: Option<CandidateAction>,
    ) {
        self.seen.push(self.matching.key(&candidate.surface));
        self.entries.push(CandidateEntry { candidate, action });
    }

    /// 字面尚未出現時才加入；回傳是否加入。
    pub(super) fn push_unique(&mut self, candidate: Candidate) -> bool {
        if self.contains(&candidate.surface) {
            return false;
        }
        self.push(candidate);
        true
    }

    /// 把 extras 插在 `anchor` 之前；有插入時整份清單去重。
    ///
    /// 注音模糊音用這條路徑：原本插在字面讀音候選前面，重複字面保留先出現
    /// 的那一項。回傳實際插入的候選數。
    pub(super) fn insert_before_unique(
        &mut self,
        anchor: &str,
        extras: impl IntoIterator<Item = Candidate>,
    ) -> usize {
        let extras: Vec<Candidate> = extras.into_iter().collect();
        if extras.is_empty() {
            return 0;
        }
        let at = self
            .entries
            .iter()
            .position(|entry| entry.candidate.surface == anchor)
            .unwrap_or(self.entries.len());
        let inserted = extras.len();
        for (offset, candidate) in extras.into_iter().enumerate() {
            let index = (at + offset).min(self.entries.len());
            self.entries.insert(
                index,
                CandidateEntry {
                    candidate,
                    action: None,
                },
            );
        }
        self.dedupe();
        inserted
    }

    /// 尾端追加 extras，跳過已存在的字面；回傳實際加入的候選數。
    pub(super) fn append_unique(&mut self, extras: impl IntoIterator<Item = Candidate>) -> usize {
        let mut added = 0;
        for candidate in extras {
            if self.push_unique(candidate) {
                added += 1;
            }
        }
        added
    }

    /// 依 [`SurfaceMatch`] 去重，保留每個字面的第一項。
    pub(super) fn dedupe(&mut self) {
        let mut seen: Vec<String> = Vec::with_capacity(self.seen.len());
        let mut entries = Vec::with_capacity(self.entries.len());
        for entry in std::mem::take(&mut self.entries) {
            let key = self.matching.key(&entry.candidate.surface);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            entries.push(entry);
        }
        self.seen = seen;
        self.entries = entries;
    }

    /// 只保留前 `limit` 項；截斷發生在尾端，不改變前面的順序。
    pub(super) fn truncate(&mut self) {
        if self.entries.len() > self.limit {
            self.entries.truncate(self.limit);
            self.seen = self
                .entries
                .iter()
                .map(|entry| self.matching.key(&entry.candidate.surface))
                .collect();
        }
    }

    /// 把顯示字移到第一項；找不到或已經是第一項時不動作。
    pub(super) fn show_first(&mut self) -> bool {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.candidate.surface == self.display)
        else {
            return false;
        };
        if index == 0 {
            return false;
        }
        let display = self.entries.remove(index);
        self.entries.insert(0, display);
        true
    }

    /// 收尾：去重、截斷、顯示字排第一，最後拆成候選與平行動作。
    pub(super) fn assemble(mut self, options: AssemblyOptions) -> AssembledCandidates {
        if options.dedupe {
            self.dedupe();
        }
        self.truncate();
        if options.display_first {
            self.show_first();
        }
        let mut candidates = Vec::with_capacity(self.entries.len());
        let mut actions = Vec::with_capacity(self.entries.len());
        for entry in self.entries {
            candidates.push(entry.candidate);
            actions.push(entry.action);
        }
        AssembledCandidates {
            candidates,
            actions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateSource;

    fn candidate(surface: &str) -> Candidate {
        Candidate::new(surface, "讀音", 0, CandidateSource::Lexicon)
    }

    /// 三個 lane 的共同契約：同樣的 hypotheses 與 extras，不論由哪個 lane
    /// 的設定組裝，順序、截斷與顯示字位置都一致。
    #[test]
    fn test_three_lanes_share_candidate_set_contract() {
        #[derive(Clone, Copy)]
        enum Lane {
            Zh,
            Ja,
            Mixed,
        }

        fn assemble(lane: Lane) -> Vec<String> {
            let hypotheses = ["顯示字", "甲", "乙", "丙", "丁"]
                .into_iter()
                .map(candidate)
                .collect::<Vec<_>>();
            let extras = [candidate("模糊"), candidate("建議")];
            let matching = match lane {
                Lane::Zh | Lane::Ja => SurfaceMatch::Exact,
                Lane::Mixed => SurfaceMatch::AsciiLower,
            };
            let mut set = CandidateSet::from_hypotheses("顯示字", 4, matching, hypotheses);
            set.insert_before_unique("顯示字", extras);
            set.append_unique([candidate("尾巴")]);
            let assembled = set.assemble(AssemblyOptions {
                dedupe: false,
                display_first: true,
            });
            assembled
                .candidates
                .into_iter()
                .map(|candidate| candidate.surface)
                .collect()
        }

        let zh = assemble(Lane::Zh);
        assert_eq!(zh, assemble(Lane::Ja), "注音與日文的共同契約");
        assert_eq!(zh, assemble(Lane::Mixed), "混打的共同契約");
        assert_eq!(zh.len(), 4, "三個 lane 都在同一個名額截斷");
        assert_eq!(
            zh.first().map(String::as_str),
            Some("顯示字"),
            "顯示字排第一"
        );
    }

    #[test]
    fn test_fuzzy_extras_stay_before_literal_and_keep_first_duplicate() {
        let mut set = CandidateSet::from_hypotheses(
            "字面",
            10,
            SurfaceMatch::Exact,
            [candidate("字面"), candidate("甲")],
        );
        let inserted = set.insert_before_unique("字面", [candidate("近音"), candidate("甲")]);
        let assembled = set.assemble(AssemblyOptions {
            dedupe: false,
            display_first: false,
        });
        let surfaces: Vec<&str> = assembled
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert_eq!(inserted, 2);
        assert_eq!(
            surfaces,
            ["近音", "甲", "字面"],
            "模糊音插在字面前，重複留第一項"
        );
    }

    #[test]
    fn test_actions_stay_parallel_after_dedupe() {
        let mut set = CandidateSet::new("", usize::MAX, SurfaceMatch::Exact);
        for surface in ["甲", "乙", "甲"] {
            set.push_with_action(
                candidate(surface),
                Some(CandidateAction {
                    insert: surface.to_string(),
                    ..CandidateAction::default()
                }),
            );
        }
        let assembled = set.assemble(AssemblyOptions {
            dedupe: true,
            display_first: false,
        });
        assert_eq!(assembled.candidates.len(), 2);
        assert_eq!(assembled.actions.len(), 2, "動作與候選逐一對齊");
        for (candidate, action) in assembled.candidates.iter().zip(&assembled.actions) {
            assert_eq!(
                Some(candidate.surface.as_str()),
                action.as_ref().map(|action| action.insert.as_str())
            );
        }
    }

    #[test]
    fn test_show_first_leaves_order_when_display_missing() {
        let set = CandidateSet::from_hypotheses(
            "缺席",
            10,
            SurfaceMatch::Exact,
            [candidate("甲"), candidate("乙")],
        );
        let assembled = set.assemble(AssemblyOptions::default());
        let surfaces: Vec<&str> = assembled
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert_eq!(surfaces, ["甲", "乙"]);
    }
}
