# w1-data-ja：jadic pack 讀取

只實作讀取端。Go 的寫入／匯出路徑（convert、import、inspect、symbols）不在本 task 範圍。

## Go → Rust 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/jadic/pack.go` | `crates/jc-data/src/ja/pack.rs` | ported |
| `internal/jadic/manifest.go` | `crates/jc-data/src/ja/manifest.rs` | ported |
| `internal/jadic/order.go` | `crates/jc-data/src/ja/order.rs` | ported |
| `internal/jadic/convert.go` | `crates/jc-data/src/ja/`（無） | n/a：匯出 words.tsv 是寫入端，本 task 不做 |
| `internal/jadic/import.go` | `crates/jc-data/src/ja/`（無） | n/a：匯入流程是寫入端 |
| `internal/jadic/inspect.go` | `crates/jc-data/src/ja/`（無） | n/a：CLI 診斷輸出 |
| `internal/jadic/symbols.go` | `crates/jc-data/src/ja/`（無） | n/a：記号分類是匯入流程的一部分 |

## 測試去向

| Go 測試 | Rust 測試 | 狀態 |
|---|---|---|
| `TestParseSamplePack` | `ja::tests::test_parse_sample_pack` | ported |
| `TestCapabilitiesV1HasNoContextCosts` | `ja::tests::test_capabilities_v1_has_no_context_costs` | ported |
| `TestDescribeReportsMissingContextCosts` | 無 | n/a：`Describe` 屬 inspect，不在本 task |
| `TestParseRejectsStructuralCorruption` | `ja::tests::test_parse_rejects_structural_corruption` | rewritten：改用 `PackError` variant 斷言 |
| `TestParseRejectsEntryCorruption` | `ja::tests::test_parse_rejects_entry_corruption` | rewritten：同上 |
| `TestParseRejectsIndexCorruption` | `ja::tests::test_parse_rejects_index_corruption` | rewritten：同上 |
| `TestParseRejectsKatakanaReading` | `ja::tests::test_parse_rejects_katakana_reading` | ported |
| `TestParseRejectsMixedReading` | `ja::tests::test_parse_rejects_mixed_reading` | ported |
| `TestParseRejectsNonKanaReading` | `ja::tests::test_parse_rejects_non_kana_reading` | ported |
| `TestParseAcceptsProlongedSoundMark` | `ja::tests::test_parse_accepts_prolonged_sound_mark` | ported |
| `packtest_test.go`（builder） | `ja::tests::{encode_test_pack, sample_pack, reseal}` | rewritten：`testing.T` 改成 `assert`／`panic` |
| `TestManifestVerify` | `ja::tests::test_manifest_verify` | ported |
| `TestManifestVerifyRejectsMismatch` | `ja::tests::test_manifest_verify_rejects_mismatch` | rewritten：改用 variant 斷言 |
| `TestLoadManifestRecordsFileHash` | `ja::tests::test_load_manifest_records_file_hash` | ported，另加 `verify_file` 改寫偵測 |
| `TestCompareEntries` | `ja::tests::test_compare_entries` | ported |
| `TestSortSurfaceEntriesStable` | `ja::tests::test_sort_surface_entries_stable` | ported |
| `TestConvertSignedCostsKeepOrder` | 無 | n/a：匯出流程，本 task 不做 |
| `TestConvertSignedCostsRankWithinSurface` | 無 | n/a：同上 |
| `TestConvertOrdersSameSurface` | 無 | n/a：同上 |
| `TestReferencePack` | `ja::tests::test_reference_pack` | ported，實測 `packs/ja-lexicon-public-core.pack` 與 manifest |
| `TestReferencePackImportStable` | 無 | n/a：匯入流程，本 task 不做 |

## 實測資料

- pack：`/mnt/work/jc/jc-ime-data/packs/ja-lexicon-public-core.pack`
- manifest：`/mnt/work/jc/jc-ime-data/packs/ja-lexicon-public-core.manifest.json`
- 測試以 `require_data!` 取得，缺資料時 skip。

## 未完成／阻塞

- `crates/jc-data/src/lib.rs` 沒有 `pub mod ja;`。依任務書邊界沒有自行加；需由 `lib.rs` 的 owner 補上，補上後 `ja` 模組才會編進 crate。
- `jc-testdata` crate 只存在於 `c0-contracts` branch，本 worktree 的 workspace 沒有它。測試暫用同語意的本地 `require_data!`；合併後應換成 `jc_testdata::require_data!`。
