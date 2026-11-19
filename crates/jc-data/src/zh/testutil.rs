//! 測試用的資料包寫入器。
//!
//! 依前身 Go `core/lexicon/compact.go` 與 `core/lm/compact.go` 的寫入順序
//! 產生最小資料包，讓讀取端能在沒有外部資料時驗證格式。

use std::collections::{BTreeSet, HashMap};

use super::format::{LEXICON_MAGIC, LEXICON_VERSION, LM_MAGIC, LM_VERSION};

/// 依序寫入區段的建構器。
pub(crate) struct Builder {
    buf: Vec<u8>,
}

impl Builder {
    pub(crate) fn new(magic: &[u8], version: u32) -> Builder {
        let mut builder = Builder { buf: Vec::new() };
        builder.buf.extend_from_slice(magic);
        builder.u32(version);
        builder
    }

    pub(crate) fn u32(&mut self, value: u32) -> &mut Builder {
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub(crate) fn u16s(&mut self, values: &[u16]) -> &mut Builder {
        self.u32(values.len() as u32);
        for value in values {
            self.buf.extend_from_slice(&value.to_le_bytes());
        }
        self
    }

    pub(crate) fn u32s(&mut self, values: &[u32]) -> &mut Builder {
        self.u32(values.len() as u32);
        for value in values {
            self.buf.extend_from_slice(&value.to_le_bytes());
        }
        self
    }

    pub(crate) fn f32s(&mut self, values: &[f32]) -> &mut Builder {
        self.u32(values.len() as u32);
        for value in values {
            self.buf.extend_from_slice(&value.to_le_bytes());
        }
        self
    }

    pub(crate) fn bytes(&mut self, values: &[u8]) -> &mut Builder {
        self.u32(values.len() as u32);
        self.buf.extend_from_slice(values);
        self
    }

    pub(crate) fn string(&mut self, value: &str) -> &mut Builder {
        self.u32(value.len() as u32);
        self.buf.extend_from_slice(value.as_bytes());
        self
    }

    pub(crate) fn finish(&self) -> Vec<u8> {
        let mut out = self.buf.clone();
        out.extend_from_slice(&crc32fast::hash(&self.buf).to_le_bytes());
        out
    }
}

/// 一筆測試詞條。
#[derive(Debug, Clone)]
pub(crate) struct Sample {
    surface: String,
    reading: Vec<String>,
    prw: f32,
    flags: u8,
}

impl Sample {
    pub(crate) fn new(surface: &str, reading: &[&str], prw: f32) -> Sample {
        Sample::flags(surface, reading, prw, 0)
    }

    pub(crate) fn flags(surface: &str, reading: &[&str], prw: f32, flags: u8) -> Sample {
        Sample {
            surface: surface.to_string(),
            reading: reading.iter().map(|unit| unit.to_string()).collect(),
            prw,
            flags,
        }
    }
}

/// 產生 `lexicon.bin`。
pub(crate) fn encode_lexicon(samples: &[Sample]) -> Vec<u8> {
    let mut unit_set = BTreeSet::new();
    for sample in samples {
        assert!(!sample.reading.is_empty(), "測試詞條的讀音不可為空");
        unit_set.extend(sample.reading.iter().cloned());
    }
    let units: Vec<String> = unit_set.into_iter().collect();
    let ids: HashMap<&str, u16> = units
        .iter()
        .enumerate()
        .map(|(index, unit)| (unit.as_str(), index as u16))
        .collect();

    let mut rows: Vec<(Vec<u16>, &Sample)> = samples
        .iter()
        .map(|sample| {
            let ids: Vec<u16> = sample
                .reading
                .iter()
                .map(|unit| ids[unit.as_str()])
                .collect();
            (ids, sample)
        })
        .collect();
    rows.sort_by(|(left_ids, left), (right_ids, right)| {
        left_ids
            .cmp(right_ids)
            .then_with(|| left.surface.cmp(&right.surface))
    });

    let mut rd: Vec<u16> = Vec::new();
    let mut rd_off = Vec::new();
    let mut sf_off = Vec::new();
    let mut sf_len = Vec::new();
    let mut rd_len = Vec::new();
    let mut flags = Vec::new();
    let mut prw = Vec::new();
    let mut strs = String::new();
    let mut seen: HashMap<&str, u32> = HashMap::new();

    let mut index = 0;
    while index < rows.len() {
        let (ids, sample) = &rows[index];
        let mut chosen = *sample;
        index += 1;
        while index < rows.len() && rows[index].0 == *ids && rows[index].1.surface == sample.surface
        {
            if rows[index].1.prw > chosen.prw {
                chosen = rows[index].1;
            }
            index += 1;
        }
        let offset = match seen.get(chosen.surface.as_str()) {
            Some(offset) => *offset,
            None => {
                let offset = strs.len() as u32;
                seen.insert(chosen.surface.as_str(), offset);
                strs.push_str(&chosen.surface);
                offset
            }
        };
        rd_off.push(rd.len() as u32);
        rd_len.push(ids.len() as u8);
        rd.extend_from_slice(ids);
        sf_off.push(offset);
        sf_len.push(chosen.surface.len() as u16);
        flags.push(chosen.flags);
        prw.push(chosen.prw);
    }

    let surfaces: Vec<&str> = (0..sf_off.len())
        .map(|index| {
            let start = sf_off[index] as usize;
            &strs[start..start + sf_len[index] as usize]
        })
        .collect();
    let mut by_text: Vec<u32> = (0..surfaces.len() as u32).collect();
    by_text.sort_by(|&left, &right| {
        surfaces[left as usize]
            .cmp(surfaces[right as usize])
            .then_with(|| left.cmp(&right))
    });

    let mut syls = String::new();
    for unit in &units {
        syls.push_str(unit);
        syls.push(' ');
    }

    let mut builder = Builder::new(LEXICON_MAGIC, LEXICON_VERSION);
    builder
        .string(&syls)
        .u16s(&rd)
        .u32s(&rd_off)
        .u32s(&sf_off)
        .u16s(&sf_len)
        .bytes(&rd_len)
        .bytes(&flags)
        .f32s(&prw)
        .string(&strs)
        .u32s(&by_text);
    builder.finish()
}

/// 產生 `lm.bin`；詞必須已依字串排序。
pub(crate) fn encode_lm(uni: &[(&str, f64)], bi: &[(&str, &str, f64)]) -> Vec<u8> {
    let words: Vec<&str> = uni.iter().map(|(word, _)| *word).collect();
    let mut strs = String::new();
    let mut off = Vec::new();
    let mut probs = Vec::new();
    for (word, probability) in uni {
        off.push(strs.len() as u32);
        strs.push_str(word);
        probs.push(*probability as f32);
    }
    off.push(strs.len() as u32);

    let mut pairs = bi.to_vec();
    pairs.sort_by_key(|(left, right, _)| {
        (
            words.iter().position(|word| word == left),
            words.iter().position(|word| word == right),
        )
    });

    let mut left_start = Vec::new();
    let mut right = Vec::new();
    let mut q = Vec::new();
    let mut cursor = 0;
    for word in &words {
        left_start.push(cursor as u32);
        for (left, target, probability) in &pairs {
            if left != word {
                continue;
            }
            let target = words
                .iter()
                .position(|candidate| candidate == target)
                .unwrap_or_else(|| panic!("bigram 後詞 {target:?} 沒有 unigram"));
            right.push((target & 0xff) as u8);
            right.push(((target >> 8) & 0xff) as u8);
            right.push(((target >> 16) & 0xff) as u8);
            let x = -probability.log10() / 8.0 * 255.0;
            q.push(x.clamp(0.0, 255.0).round() as u8);
            cursor += 1;
        }
    }
    left_start.push(cursor as u32);

    let mut builder = Builder::new(LM_MAGIC, LM_VERSION);
    builder
        .string(&strs)
        .u32s(&off)
        .f32s(&probs)
        .u32s(&left_start)
        .bytes(&right)
        .bytes(&q);
    builder.finish()
}
