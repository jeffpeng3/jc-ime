//! 語言後端：把 [`AutoRun`] 包成現有解碼器。
//!
//! 後端只看得到一段連續的 `Auto` 讀音與前後文脈，不知道 `User` 片段長什麼
//! 樣子；所有權由 [`crate::composition::Composition`] 決定。
//!
//! 注音後端用 [`Lattice`] 與 [`Decoder`] 解一段讀音；日文後端用
//! [`crate::ja::decode::JaDecoder`] 做統計轉換。日文的逐字変換控制器另外
//! 負責停頓、神經門檻與 async 補正，但固定的片段仍由 `Composition` 提供。

use std::sync::Arc;

use super::*;
use crate::composition::{AutoRun, ConvertBackend, ConvertContext, ConvertError, Converted, Lang};
use crate::decode::Locks;

/// 注音轉換後端。
pub struct ZhConvertBackend {
    lexicon: InMemoryLexicon,
    ctx: ScoreContext,
    lm: Option<Arc<crate::score::ZhLm>>,
}

impl ZhConvertBackend {
    /// 建立後端。
    pub fn new(
        lexicon: InMemoryLexicon,
        ctx: ScoreContext,
        lm: Option<Arc<crate::score::ZhLm>>,
    ) -> Self {
        Self { lexicon, ctx, lm }
    }
}

impl ConvertBackend for ZhConvertBackend {
    fn convert(
        &mut self,
        run: &AutoRun,
        _ctx: &ConvertContext<'_>,
    ) -> Result<Vec<Converted>, ConvertError> {
        if run.lang != Lang::Zh {
            return Err(ConvertError::Unsupported(run.lang));
        }
        let units: Vec<&str> = run
            .readings
            .iter()
            .flat_map(|reading| reading.split_whitespace())
            .collect();
        if units.is_empty() {
            return Err(ConvertError::Empty);
        }
        let lattice = Lattice::build(&units, &self.lexicon, &self.ctx)
            .map_err(|error| ConvertError::Backend(error.to_string()))?;
        let scorer = super::core::zh_scorer(&self.ctx, self.lm.as_deref());
        let decoder = super::core::zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
        let converted: Vec<Converted> = segs
            .iter()
            .map(|seg| {
                let from = seg.start.min(units.len());
                let to = seg.end.min(units.len()).max(from);
                Converted::new(
                    units[from..to].join(" "),
                    seg.word.clone(),
                    to - from,
                    Lang::Zh,
                )
            })
            .collect();
        if converted.is_empty() {
            Err(ConvertError::Empty)
        } else {
            Ok(converted)
        }
    }
}

/// 日文統計轉換後端。
pub struct JaConvertBackend {
    decoder: Arc<crate::ja::decode::JaDecoder>,
}

impl JaConvertBackend {
    /// 建立後端。
    pub fn new(decoder: Arc<crate::ja::decode::JaDecoder>) -> Self {
        Self { decoder }
    }
}

impl ConvertBackend for JaConvertBackend {
    fn convert(
        &mut self,
        run: &AutoRun,
        ctx: &ConvertContext<'_>,
    ) -> Result<Vec<Converted>, ConvertError> {
        if run.lang != Lang::Ja {
            return Err(ConvertError::Unsupported(run.lang));
        }
        let context = crate::ja::decode::JaContext {
            left: ctx.left.to_string(),
            right: ctx.right.to_string(),
        };
        // 明確走 `ja::live::Converter`：`JaDecoder` 另外有回傳 `Option` 的
        // 便利方法，名字相同。
        let result =
            crate::ja::live::Converter::convert(self.decoder.as_ref(), &run.reading, &context)
                .map_err(|error| ConvertError::Backend(error.to_string()))?;
        let converted: Vec<Converted> = result
            .clauses
            .iter()
            .map(|clause| {
                Converted::new(
                    clause.reading.clone(),
                    clause.surface.clone(),
                    clause.reading.chars().count(),
                    Lang::Ja,
                )
            })
            .collect();
        if converted.is_empty() {
            Err(ConvertError::Empty)
        } else {
            Ok(converted)
        }
    }
}

impl Session {
    /// 把目前組字裡的每個 `Auto` 區段交給對應語言後端轉換。
    ///
    /// 回傳每個區段與結果，順序與 [`Session::auto_runs`] 相同。`User` 片段
    /// 不會出現在回傳值裡，因此不可能被任何後端改寫。
    pub fn convert_auto_runs(&self) -> Vec<(AutoRun, Result<Vec<Converted>, ConvertError>)> {
        let mut zh =
            ZhConvertBackend::new(self.lexicon.clone(), self.ctx.clone(), self.zh_lm.clone());
        let mut ja = self.config.ja.clone().map(JaConvertBackend::new);
        self.composition
            .auto_runs()
            .into_iter()
            .map(|run| {
                let ctx = self.composition.context_for(&run);
                let result = match run.lang {
                    Lang::Zh => zh.convert(&run, &ctx),
                    Lang::Ja => match ja.as_mut() {
                        Some(backend) => backend.convert(&run, &ctx),
                        None => Err(ConvertError::Backend("沒有日文解碼器".to_string())),
                    },
                    Lang::Literal => Err(ConvertError::Unsupported(run.lang)),
                };
                (run, result)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::{Composition, Owner};
    use crate::lattice::{InMemoryLexicon, LexEntry};

    fn lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("台北", "ㄊㄞˊ ㄅㄟˇ", 0, 0));
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        lex.push(LexEntry::new("北", "ㄅㄟˇ", 0, 0));
        lex
    }

    #[test]
    fn zh_backend_converts_only_the_given_run() {
        let mut backend = ZhConvertBackend::new(lexicon(), ScoreContext::default(), None);
        let run = AutoRun {
            span: 0..2,
            reading: "ㄊㄞˊ ㄅㄟˇ".to_string(),
            readings: vec!["ㄊㄞˊ".to_string(), "ㄅㄟˇ".to_string()],
            units: 2,
            lang: Lang::Zh,
            boundaries: vec![1],
        };
        let converted = backend
            .convert(
                &run,
                &ConvertContext {
                    left: "我",
                    right: "",
                },
            )
            .expect("注音轉換");
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].text, "台北");
        assert_eq!(converted[0].units, 2);
        assert_eq!(converted[0].lang, Lang::Zh);
    }

    #[test]
    fn backend_rejects_other_language() {
        let mut backend = ZhConvertBackend::new(lexicon(), ScoreContext::default(), None);
        let run = AutoRun {
            span: 0..1,
            reading: "あ".to_string(),
            readings: vec!["あ".to_string()],
            units: 1,
            lang: Lang::Ja,
            boundaries: Vec::new(),
        };
        assert_eq!(
            backend
                .convert(
                    &run,
                    &ConvertContext {
                        left: "",
                        right: ""
                    }
                )
                .unwrap_err(),
            ConvertError::Unsupported(Lang::Ja)
        );
    }

    /// 轉換只套用到 Auto 區段；User 片段原樣留在結果裡。
    #[test]
    fn only_auto_runs_are_sent() {
        let mut composition = Composition::new();
        composition.push_auto("台", "ㄊㄞˊ", 1, Lang::Zh);
        composition.push_auto("北", "ㄅㄟˇ", 1, Lang::Zh);
        composition.mark_user(0..1);
        let segments = composition.segments().to_vec();
        let mut backend = ZhConvertBackend::new(lexicon(), ScoreContext::default(), None);
        let mut out = String::new();
        for segment in &segments {
            if segment.owner == Owner::User {
                out.push_str(&composition.text()[segment.span.start..segment.span.end]);
            }
        }
        let runs = composition.auto_runs();
        assert_eq!(runs.len(), 1);
        let ctx = composition.context_for(&runs[0]);
        let converted = backend.convert(&runs[0], &ctx).expect("轉換 Auto");
        out.push_str(
            &converted
                .iter()
                .map(|item| item.text.as_str())
                .collect::<String>(),
        );
        assert_eq!(out, "台北");
        assert!(segments.iter().any(|segment| segment.owner == Owner::User));
        // 只改 Auto 的讀音，不會動到 User 片段的 span。
        let user = segments
            .iter()
            .find(|segment| segment.owner == Owner::User)
            .expect("有 User 片段");
        assert_eq!(user.span, 0..1);
        assert_eq!(user.reading, "ㄊㄞˊ");
    }

    #[test]
    fn composition_reports_three_languages() {
        let mut composition = Composition::new();
        composition.push_auto("台", "ㄊㄞˊ", 1, Lang::Zh);
        composition.push_auto("東京", "とうきょう", 5, Lang::Ja);
        composition.push_auto("abc", "", 0, Lang::Literal);
        composition.push_auto("北", "ㄅㄟˇ", 1, Lang::Zh);
        // 使用者固定中間的日文片段。
        composition.mark_user(1..3);
        let runs = composition.auto_runs();
        assert_eq!(runs.len(), 2, "User 片段切開兩段 Auto");
        assert_eq!(runs[0].lang, Lang::Zh);
        assert_eq!(runs[0].reading, "ㄊㄞˊ");
        assert_eq!(runs[1].lang, Lang::Zh);
        assert_eq!(runs[1].reading, "ㄅㄟˇ");
        assert!(composition
            .segments()
            .iter()
            .any(|segment| segment.owner == Owner::User && segment.lang == Lang::Ja));
    }
}
