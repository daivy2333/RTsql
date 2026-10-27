#[path = "support/index_key_encoding.rs"]
mod index_key_encoding;

use index_key_encoding::{
    byte_cmp, decode_ordered_int, decode_ordered_str, encode_for_layout, encode_ordered_int,
    encode_ordered_str, fixed32_key_bytes, measure_file_reads, pack_prefix_ints, pack_records,
    page_free_space, prefix_slot_prefix_len, read_prefix_int_slot, read_slot, records_for,
    values_common_string, values_long_string, values_short_int, values_symmetric_int,
    values_wide_int, write_pages, CandidateError, CountingStorage, Domain, Layout, LogicalValue,
    Record, Tail, TailKind, HOT_SET_LEN, WORKLOAD_LEN,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rtsql::storage::page_format::{SlottedPage, SlottedPageRef};
use rtsql::storage::{Key, Page, PageId};
use rtsql::Value;

const SIGNED_BOUNDS: [i64; 9] = [i64::MIN, i64::MAX, -256, -255, -1, 0, 1, 255, 256];

#[test]
fn integer_candidate_preserves_signed_order() {
    let mut numeric = SIGNED_BOUNDS.to_vec();
    numeric.sort();

    let mut by_candidate = SIGNED_BOUNDS.to_vec();
    by_candidate.sort_by(|a, b| {
        let left = encode_for_layout(Layout::OrderedVariable, &LogicalValue::Int(*a)).unwrap();
        let right = encode_for_layout(Layout::OrderedVariable, &LogicalValue::Int(*b)).unwrap();
        byte_cmp(&left, &right)
    });
    assert_eq!(
        by_candidate, numeric,
        "候选整数编码顺序应与 i64 数值顺序一致"
    );

    let mut seen = Vec::new();
    for value in SIGNED_BOUNDS {
        let encoded = encode_ordered_int(value);
        let via_layout = encode_for_layout(Layout::Fixed32, &LogicalValue::Int(value)).unwrap();
        let fixed = fixed32_key_bytes(&LogicalValue::Int(value)).unwrap();
        assert_eq!(via_layout.as_slice(), fixed.as_slice());
        assert!(!encoded.is_empty(), "编码不得为空: {value}");
        let decoded = decode_ordered_int(&encoded).expect("整数应能回读");
        assert_eq!(decoded, value);
        assert!(!seen.contains(&encoded), "不同整数不得碰撞: {value}");
        seen.push(encoded);
    }

    let mut by_fixed = SIGNED_BOUNDS.to_vec();
    by_fixed.sort_by(|a, b| {
        let ka = fixed32_key_bytes(&LogicalValue::Int(*a)).expect("fixed32 整数");
        let kb = fixed32_key_bytes(&LogicalValue::Int(*b)).expect("fixed32 整数");
        ka.cmp(&kb)
    });
    let mut by_be = SIGNED_BOUNDS.to_vec();
    by_be.sort_by_key(|value| value.to_be_bytes());
    assert_eq!(by_fixed, by_be, "旧 fixed32 只要求大端字节序");
    assert_ne!(by_fixed, numeric, "旧大端字节序不是有符号数值顺序");
    let neg = fixed32_key_bytes(&LogicalValue::Int(-1)).unwrap();
    let pos = fixed32_key_bytes(&LogicalValue::Int(1)).unwrap();
    assert!(neg > pos);
}

#[test]
fn string_candidate_preserves_full_value_and_lexical_order() {
    let mut samples = vec![
        String::new(),
        "a".to_string(),
        "ab".to_string(),
        "b".to_string(),
        "prefix".to_string(),
        "prefix-shared".to_string(),
        "alpine".to_string(),
        "你".to_string(),
        "你好".to_string(),
        "a\0".to_string(),
        "a\0b".to_string(),
        "a\0\0".to_string(),
        "\0".to_string(),
        "\0a".to_string(),
        "trailing\0".to_string(),
    ];
    samples.extend(random_byte_strings(38, 64));
    samples.sort();
    samples.dedup();

    let mut seen = Vec::new();
    for value in &samples {
        let encoded = encode_ordered_str(value).expect("字符串应可编码");
        let decoded = decode_ordered_str(&encoded).expect("字符串应能回读");
        assert_eq!(&decoded, value, "回读必须保留全部字节");
        assert!(
            !seen.iter().any(|prev| prev == &encoded),
            "不同字符串不得碰撞: {value:?}"
        );
        seen.push(encoded);

        match fixed32_key_bytes(&LogicalValue::Str(value.clone())) {
            Ok(key) => {
                let escaped = encode_ordered_str(value).unwrap();
                assert!(escaped.len() <= 32);
                let round = decode_fixed32_string(&key);
                assert_eq!(round, *value, "fixed32 必须按完整转义回读，不能按尾零裁剪");
                let inferred = Key::deserialize(&key);
                if value.as_bytes().contains(&0) || value.is_empty() {
                    assert_ne!(
                        inferred.as_bytes(),
                        escaped.as_slice(),
                        "尾零推断不能当作完整字符串编码"
                    );
                }
            }
            Err(index_key_encoding::CandidateError::Unsupported) => {
                let escaped = encode_ordered_str(value).unwrap();
                assert!(escaped.len() > 32, "只有转义超过 32 字节才是不支持");
            }
            Err(other) => panic!("fixed32 字符串只允许成功或不支持，得到 {other:?}"),
        }
    }

    let mut by_text = samples.clone();
    by_text.sort();
    let mut by_enc = samples.clone();
    by_enc.sort_by(|a, b| {
        encode_ordered_str(a)
            .unwrap()
            .cmp(&encode_ordered_str(b).unwrap())
    });
    assert_eq!(by_enc, by_text, "候选字符串顺序应与 Rust 字符串字典序一致");
}

#[test]
fn prefix_restart_reconstructs_complete_keys() {
    let values: Vec<i64> = (1..=18).collect();
    let packed = pack_records(
        Layout::PrefixRestart,
        &values
            .iter()
            .enumerate()
            .map(|(index, value)| Record {
                value: LogicalValue::Int(*value),
                tail: Tail::leaf(rtsql::storage::RowId::new(index as u32, index as u16)),
            })
            .collect::<Vec<_>>(),
    )
    .expect("18 个小整数应能装页");
    assert_eq!(packed.slot_bytes, packed.locations.len() as u64 * 6);
    assert!(packed.payload_bytes >= packed.slot_bytes);
    let pages = packed.pages;
    assert_eq!(pages.len(), 1, "18 条小记录应落在同一页");
    let page = &pages[0];

    assert_eq!(prefix_slot_prefix_len(page, 0).unwrap(), 0);
    assert_ne!(
        prefix_slot_prefix_len(page, 15).unwrap(),
        0,
        "第 16 条仍在本组"
    );
    assert_eq!(prefix_slot_prefix_len(page, 16).unwrap(), 0, "每 16 条重启");
    assert_ne!(
        prefix_slot_prefix_len(page, 17).unwrap(),
        0,
        "重启后的下一条重新取前缀"
    );

    for (index, value) in values.iter().enumerate() {
        let got = read_prefix_int_slot(page, index).expect("应能重建完整键");
        assert_eq!(got, *value);
        let rebuilt = encode_ordered_int(got);
        assert_eq!(rebuilt, encode_ordered_int(*value));
    }

    let mut order = values.clone();
    order.sort_by(|a, b| byte_cmp(&encode_ordered_int(*a), &encode_ordered_int(*b)));
    assert_eq!(order, values);

    let mut broken_prev = page.clone();
    corrupt_slot_payload(&mut broken_prev, 0);
    assert!(read_prefix_int_slot(&broken_prev, 15).is_err());
    assert_eq!(read_prefix_int_slot(&broken_prev, 16).unwrap(), values[16]);

    let mut broken_restart = page.clone();
    corrupt_slot_payload(&mut broken_restart, 16);
    assert_eq!(
        read_prefix_int_slot(&broken_restart, 15).unwrap(),
        values[15]
    );
    assert!(read_prefix_int_slot(&broken_restart, 17).is_err());

    let many: Vec<i64> = (1..=300).collect();
    let multi = pack_prefix_ints(&many).expect("多页");
    assert!(multi.len() >= 2, "300 条应跨页");
    let page1_count = SlottedPageRef::new(&multi[0]).slot_count();
    assert!(page1_count < many.len());
    assert_eq!(prefix_slot_prefix_len(&multi[1], 0).unwrap(), 0);
    assert_eq!(
        read_prefix_int_slot(&multi[1], 0).unwrap(),
        many[page1_count],
        "新页首条必须是重启项，且只依赖本页"
    );
}

#[test]
fn fixed32_matches_current_key_bytes() {
    for value in SIGNED_BOUNDS {
        let got = fixed32_key_bytes(&LogicalValue::Int(value)).expect("fixed32 整数");
        let via_key = *Key::new(&value.to_be_bytes()).full_data();
        let via_value = *Value::Int(value).to_key().unwrap().full_data();
        assert_eq!(got, via_key);
        assert_eq!(got, via_value);
        assert_eq!(&got[..8], &value.to_be_bytes());
        assert!(got[8..].iter().all(|byte| *byte == 0));
    }

    let too_long = "x".repeat(31);
    let err = fixed32_key_bytes(&LogicalValue::Str(too_long)).unwrap_err();
    assert_eq!(err, CandidateError::Unsupported);

    let fits = "y".repeat(30);
    let key = fixed32_key_bytes(&LogicalValue::Str(fits.clone())).expect("30 字节可放入");
    assert_eq!(
        index_key_encoding::decode_fixed32_string(&key).unwrap(),
        fits
    );
}

#[test]
fn candidate_page_roundtrip_checks_lengths_and_capacity() {
    for (kind, variable_len, prefix_len, fixed_count) in [
        (TailKind::Leaf, 32, 30, 93usize),
        (TailKind::Internal, 34, 32, 98),
    ] {
        let variable = exact_fill_records(variable_len, 85, kind);
        let variable_pages = expect_exact_then_spill(Layout::OrderedVariable, &variable);

        let prefix = distinct_first_byte_records(prefix_len, 85, kind);
        let prefix_pages = expect_exact_then_spill(Layout::PrefixRestart, &prefix);
        assert_eq!(page_free_space(&prefix_pages[0]).unwrap(), 0);

        let fixed = (1..=fixed_count)
            .map(|i| record(LogicalValue::Int(i as i64), kind, i))
            .collect::<Vec<_>>();
        let fixed_pages = pack_records(Layout::Fixed32, &fixed).unwrap();
        assert!(fixed_pages.pages.len() >= 2);
        assert_roundtrip(Layout::Fixed32, &fixed, &fixed_pages);
        let free = page_free_space(&fixed_pages.pages[0]).unwrap();
        let first_len = slot_len(&fixed_pages.pages[0], 0);
        assert!(free < first_len + 6, "旧页装不下同长度的下一条");
        let _ = variable_pages;
    }

    let too_wide = record(LogicalValue::Str("z".repeat(4096)), TailKind::Leaf, 0);
    assert_eq!(
        pack_records(Layout::Fixed32, std::slice::from_ref(&too_wide)).unwrap_err(),
        CandidateError::Unsupported
    );
    assert_eq!(
        pack_records(Layout::OrderedVariable, std::slice::from_ref(&too_wide)).unwrap_err(),
        CandidateError::RecordDoesNotFit
    );
    assert_eq!(
        pack_records(Layout::PrefixRestart, std::slice::from_ref(&too_wide)).unwrap_err(),
        CandidateError::RecordDoesNotFit
    );
    let huge = record(
        LogicalValue::Str("z".repeat(1024 * 1024)),
        TailKind::Leaf,
        0,
    );
    assert_eq!(
        pack_records(Layout::OrderedVariable, std::slice::from_ref(&huge)).unwrap_err(),
        CandidateError::LengthExceedsU16
    );
    assert_eq!(
        pack_records(Layout::PrefixRestart, std::slice::from_ref(&huge)).unwrap_err(),
        CandidateError::LengthExceedsU16
    );

    let mut short = vec![0xFF, 0x7F];
    short.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
    let page = page_with_payload(&short);
    assert!(read_slot(
        Layout::OrderedVariable,
        Domain::Int,
        TailKind::Leaf,
        &page,
        0
    )
    .is_err());

    let packed = pack_records(
        Layout::Fixed32,
        &[record(LogicalValue::Int(1), TailKind::Leaf, 0)],
    )
    .unwrap();
    let mut corrupted = packed.pages[0].clone();
    let slot_at = corrupted.len() - 6;
    corrupted[slot_at + 4] = 0xFF;
    corrupted[slot_at + 5] = 0xFF;
    assert!(read_slot(Layout::Fixed32, Domain::Int, TailKind::Leaf, &corrupted, 0).is_err());

    let mut prefix_page = pack_prefix_ints(&[1, 2, 3]).unwrap().remove(0);
    prefix_page[16] = 1;
    prefix_page[17] = 0;
    assert!(matches!(
        read_prefix_int_slot(&prefix_page, 0),
        Err(CandidateError::LostRestart)
    ));

    let restart_lost = pack_prefix_ints(&[1, 2]).unwrap().remove(0);
    let second = SlottedPageRef::new(&restart_lost).get_slot(1).unwrap();
    let mut lost = restart_lost;
    lost[second.offset as usize] = 0xE8;
    lost[second.offset as usize + 1] = 0x03;
    assert!(read_prefix_int_slot(&lost, 1).is_err());
    assert_eq!(read_prefix_int_slot(&lost, 0).unwrap(), 1);
}

#[tokio::test]
async fn cache_counts_observe_real_storage_reads() {
    let records = (1..=200)
        .map(|i| record(LogicalValue::Int(i), TailKind::Leaf, i as usize))
        .collect::<Vec<_>>();
    let packed = pack_records(Layout::Fixed32, &records).unwrap();
    assert!(packed.pages.len() >= 2);
    assert!(packed.pages.len() <= 16);

    let dir = tempfile::tempdir().unwrap();
    let storage = std::sync::Arc::new(CountingStorage::open(&dir.path().join("pages")).unwrap());
    let ids = write_pages(&storage, &packed.pages).await.unwrap();
    storage.reset_reads();
    let pool = rtsql::storage::BufferPool::new(16, storage.clone()).unwrap();

    for (index, loc) in packed.locations.iter().enumerate() {
        let guard = pool.get_page(ids[loc.page_index]).await.unwrap();
        let view = guard.page_data();
        let (value, tail) = read_slot(
            Layout::Fixed32,
            Domain::Int,
            TailKind::Leaf,
            &view,
            loc.slot_index,
        )
        .unwrap();
        assert_eq!(value, records[index].value);
        assert_eq!(tail, records[index].tail);
    }
    let reads = storage.read_count();
    let requests = packed.locations.len() as u64;
    assert!(requests >= reads);
    assert_eq!(reads, packed.pages.len() as u64, "冷读每页恰好一次底层读");
    assert_eq!(requests - reads, requests - packed.pages.len() as u64);

    let before = storage.read_count();
    {
        let guard = pool.get_page(ids[0]).await.unwrap();
        let view = guard.page_data();
        let _ = read_slot(Layout::Fixed32, Domain::Int, TailKind::Leaf, &view, 0).unwrap();
    }
    assert_eq!(storage.read_count(), before, "命中同一缓存页不再 read_page");
}

#[tokio::test]
async fn page_metrics_cover_equal_inputs() {
    let common = values_common_string();
    assert_eq!(common.len(), WORKLOAD_LEN);
    for value in &common {
        let LogicalValue::Str(text) = value else {
            panic!("common-string 样本应为字符串");
        };
        assert!(text.len() == 8 || text.len() == 24, "{}", text.len());
        assert!(encode_ordered_str(text).unwrap().len() <= 32);
    }

    let workloads = [
        ("short-int", values_short_int(), true),
        ("symmetric-int", values_symmetric_int(), true),
        ("wide-int", values_wide_int(), true),
        ("common-string", common, true),
        ("long-string-32", values_long_string(32), false),
        ("long-string-64", values_long_string(64), false),
        ("long-string-256", values_long_string(256), false),
    ];
    for (name, values, fixed32_fits) in workloads {
        for kind in [TailKind::Leaf, TailKind::Internal] {
            let records = records_for(&values, kind);
            for layout in [
                Layout::Fixed32,
                Layout::OrderedVariable,
                Layout::PrefixRestart,
            ] {
                match pack_records(layout, &records) {
                    Err(err) => {
                        println!(
                            "layout={layout:?} workload={name} tail={kind:?} unaccommodated={err:?}"
                        );
                        if layout == Layout::Fixed32 && !fixed32_fits {
                            assert_eq!(err, CandidateError::Unsupported);
                        } else {
                            panic!("{layout:?} {name} {kind:?} 不应拒绝完整负载: {err:?}");
                        }
                    }
                    Ok(packed) => {
                        if layout == Layout::Fixed32 {
                            assert!(fixed32_fits, "{name} 不应被 fixed32 装下");
                        }
                        assert_roundtrip(layout, &records, &packed);
                        let headers = packed.pages.len() as u64 * 16;
                        println!(
                            "layout={layout:?} workload={name} tail={kind:?} pages={} payload_bytes={} slot_bytes={} header_bytes={} occupied_bytes={}",
                            packed.pages.len(),
                            packed.payload_bytes,
                            packed.slot_bytes,
                            headers,
                            packed.payload_bytes + packed.slot_bytes + headers
                        );
                        let reports = measure_file_reads(&packed, &records, layout, &[16, 64])
                            .await
                            .unwrap_or_else(|err| panic!("测量失败 {layout:?} {name}: {err}"));
                        for (capacity, report) in [16usize, 64].into_iter().zip(reports) {
                            for (phase, stats) in [
                                ("sequential", &report.sequential),
                                ("hot-warmup", &report.hot_warmup),
                                ("hot-repeat", &report.hot_repeat),
                            ] {
                                println!(
                                    "layout={layout:?} workload={name} tail={kind:?} cap={capacity} phase={phase} requests={} reads={} hits={}",
                                    stats.requests, stats.reads, stats.hits
                                );
                                assert!(stats.requests >= stats.reads);
                                assert_eq!(stats.hits + stats.reads, stats.requests);
                            }
                            assert_eq!(report.sequential.requests, records.len() as u64);
                            let hot = HOT_SET_LEN.min(records.len()) as u64;
                            assert_eq!(report.hot_warmup.requests, hot);
                            assert_eq!(report.hot_repeat.requests, hot * 4);
                            if packed.pages.len() <= capacity {
                                assert_eq!(
                                    report.sequential.reads,
                                    packed.pages.len() as u64,
                                    "容量足够时冷扫描每页只读一次"
                                );
                                assert_eq!(
                                    report.hot_repeat.reads, 0,
                                    "预热后的热点重复不应再 read_page"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

fn decode_fixed32_string(key: &[u8; 32]) -> String {
    index_key_encoding::decode_fixed32_string(key).expect("fixed32 字符串回读")
}

fn random_byte_strings(seed: u64, count: usize) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..count)
        .map(|_| {
            let len = rng.gen_range(0..48);
            let bytes = (0..len)
                .map(|_| {
                    if rng.gen_bool(0.15) {
                        0
                    } else {
                        rng.gen_range(1..128)
                    }
                })
                .collect::<Vec<u8>>();
            String::from_utf8(bytes).unwrap()
        })
        .collect()
}

fn record(value: LogicalValue, kind: TailKind, index: usize) -> Record {
    let tail = match kind {
        TailKind::Leaf => Tail::leaf(rtsql::storage::RowId::new(index as u32, index as u16)),
        TailKind::Internal => Tail::Internal(index as u32),
    };
    Record { value, tail }
}

fn exact_fill_records(len: usize, count: usize, kind: TailKind) -> Vec<Record> {
    (0..count)
        .map(|index| record(LogicalValue::Str("q".repeat(len)), kind, index))
        .collect()
}

fn distinct_first_byte_records(len: usize, count: usize, kind: TailKind) -> Vec<Record> {
    (0..count)
        .map(|index| {
            let mut bytes = vec![b'x'; len];
            bytes[0] = b'!' + index as u8;
            record(
                LogicalValue::Str(String::from_utf8(bytes).unwrap()),
                kind,
                index,
            )
        })
        .collect()
}

fn expect_exact_then_spill(layout: Layout, full: &[Record]) -> Vec<Vec<u8>> {
    let packed = pack_records(layout, full).unwrap();
    assert_eq!(packed.pages.len(), 1, "{layout:?} 应恰好装满一页");
    assert_eq!(page_free_space(&packed.pages[0]).unwrap(), 0);
    assert_roundtrip(layout, full, &packed);
    let mut spilled = full.to_vec();
    spilled.push(record(
        LogicalValue::Str("spill-record-extra-bytes!!".to_string()),
        full[0].tail.kind(),
        full.len(),
    ));
    let again = pack_records(layout, &spilled).unwrap();
    assert_eq!(again.pages.len(), 2);
    assert_eq!(page_free_space(&again.pages[0]).unwrap(), 0);
    assert_eq!(SlottedPageRef::new(&again.pages[1]).slot_count(), 1);
    assert_roundtrip(layout, &spilled, &again);
    again.pages
}

fn assert_roundtrip(layout: Layout, records: &[Record], packed: &index_key_encoding::PackedLoad) {
    let domain = match records[0].value {
        LogicalValue::Int(_) => Domain::Int,
        LogicalValue::Str(_) => Domain::Str,
    };
    let kind = records[0].tail.kind();
    for (index, loc) in packed.locations.iter().enumerate() {
        let (value, tail) = read_slot(
            layout,
            domain,
            kind,
            &packed.pages[loc.page_index],
            loc.slot_index,
        )
        .unwrap_or_else(|err| panic!("{layout:?} 槽 {index} 回读失败: {err:?}"));
        assert_eq!(value, records[index].value);
        assert_eq!(tail, records[index].tail);
    }
}

fn slot_len(page: &[u8], slot_index: usize) -> usize {
    SlottedPageRef::new(page)
        .get_slot(slot_index)
        .unwrap()
        .length as usize
}

fn page_with_payload(payload: &[u8]) -> Vec<u8> {
    let mut page = Page::new(PageId(0));
    SlottedPage::init(&mut page, index_key_encoding::PROTO_PAGE_TYPE)
        .add_slot(payload)
        .unwrap();
    page.data.to_vec()
}

fn corrupt_slot_payload(page: &mut [u8], slot_index: usize) {
    use rtsql::storage::page_format::SlottedPageRef;
    let slot = SlottedPageRef::new(page)
        .get_slot(slot_index)
        .expect("槽存在");
    let at = slot.offset as usize;
    page[at] ^= 0xFF;
}
