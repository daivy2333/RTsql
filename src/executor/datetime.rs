//! Calendar math for the DATE/TIMESTAMP value types (MS13 Iteration 000 T1).
//!
//! Pure functions only: civil-date conversion (Howard Hinnant's public-domain
//! algorithms), fixed-width parsing/formatting (DA5 display forms), timestamp
//! truncation, and the write-boundary coercion for datetime-typed columns.
//! `Value::Date` counts days since 0001-01-01 (i32); `Value::Timestamp` counts
//! microseconds since the Unix epoch (i64). No chrono/time dependency and no
//! panic paths — every fallible operation returns `Option`/`Result`.

use crate::Value;

/// Days from 0001-01-01 (the `Value::Date` epoch) to 1970-01-01 (the Unix
/// epoch), derived from the civil conversion (`days_from_civil(1970,1,1)==0`)
/// rather than hand-copied.
pub(crate) const DAYS_0001_TO_1970: i64 = -days_from_civil(1, 1, 1);

pub(crate) const MICROS_PER_SECOND: i64 = 1_000_000;
const MICROS_PER_MINUTE: i64 = 60 * MICROS_PER_SECOND;
const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MINUTE;
const MICROS_PER_DAY: i64 = 24 * MICROS_PER_HOUR;

/// Truncation unit for `trunc_ts` (DA8 unit set). Year/Day serve the
/// CAST(ts AS DATE) truncation path; the full set serves `date_trunc` and
/// `datediff` (MS13 Iteration 001 T6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TruncUnit {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}

/// Parse a `date_trunc`/`datediff` unit string (case-insensitive, DA9
/// singular forms).
pub(crate) fn parse_trunc_unit(s: &str) -> Option<TruncUnit> {
    match s.to_ascii_lowercase().as_str() {
        "year" => Some(TruncUnit::Year),
        "month" => Some(TruncUnit::Month),
        "day" => Some(TruncUnit::Day),
        "hour" => Some(TruncUnit::Hour),
        "minute" => Some(TruncUnit::Minute),
        "second" => Some(TruncUnit::Second),
        _ => None,
    }
}

/// Days since 1970-01-01 for the proleptic Gregorian date `(y, m, d)`.
const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of `days_from_civil`: `(year, month, day)` for days since
/// 1970-01-01.
const fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Parse a strict `YYYY-MM-DD` date (4-2-2 fixed width, proleptic Gregorian,
/// years 0001..9999) into days since 0001-01-01.
pub(crate) fn parse_date(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year = digits4(&b[0..4])?;
    let month = digits2(&b[5..7])?;
    let day = digits2(&b[8..10])?;
    if year == 0 || month == 0 || month > 12 {
        return None;
    }
    let day_max = days_in_month(year, month);
    if day == 0 || day > day_max {
        return None;
    }
    let serial = days_from_civil(year, month as i64, day as i64) + DAYS_0001_TO_1970;
    Some(serial as i32)
}

/// Format days-since-0001-01-01 as `YYYY-MM-DD`.
pub(crate) fn format_date(days: i32) -> String {
    let (y, m, d) = date_fields(days);
    if y >= 0 {
        format!("{:04}-{:02}-{:02}", y, m, d)
    } else {
        format!("-{:04}-{:02}-{:02}", -y, m, d)
    }
}

/// Parse `YYYY-MM-DD[ T]HH:MM:SS[.f{1..6}]` into microseconds since the Unix
/// epoch. A pure date is midnight; a timezone suffix or a fraction wider than
/// 6 digits is rejected.
pub(crate) fn parse_timestamp(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let date = parse_date(s.get(0..10)?)?;
    if b.len() == 10 {
        return Some(date_serial_to_unix_micros(date, 0, 0, 0, 0));
    }
    if b.len() < 19 || b.len() > 26 {
        return None;
    }
    if b[10] != b' ' && b[10] != b'T' {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let hour = digits2(&b[11..13])? as i64;
    let minute = digits2(&b[14..16])? as i64;
    let second = digits2(&b[17..19])? as i64;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut micros = 0u32;
    if b.len() > 19 {
        if b[19] != b'.' {
            return None;
        }
        let frac = &b[20..];
        if frac.is_empty() || frac.len() > 6 {
            return None;
        }
        let mut value = 0u32;
        for &c in frac {
            if !c.is_ascii_digit() {
                return None;
            }
            value = value * 10 + u32::from(c - b'0');
        }
        for _ in 0..(6 - frac.len()) {
            value *= 10;
        }
        micros = value;
    }
    Some(date_serial_to_unix_micros(
        date,
        hour as u32,
        minute as u32,
        second as u32,
        micros,
    ))
}

/// Format microseconds since the Unix epoch as `YYYY-MM-DD HH:MM:SS`, with a
/// 6-digit fraction appended only when micros != 0 (DA5).
pub(crate) fn format_timestamp(micros: i64) -> String {
    let (y, mo, d, h, mi, s, us) = ts_fields(micros);
    let base = format!(
        "{}{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        if y < 0 { "-" } else { "" },
        y.abs(),
        mo,
        d,
        h,
        mi,
        s
    );
    if us != 0 {
        format!("{}.{:06}", base, us)
    } else {
        base
    }
}

/// `(year, month, day)` for days since 0001-01-01.
pub(crate) fn date_fields(days: i32) -> (i32, u32, u32) {
    let unix_days = days as i64 - DAYS_0001_TO_1970;
    let (y, m, d) = civil_from_days(unix_days);
    (y as i32, m, d)
}

/// `(year, month, day, hour, minute, second, micros)` for microseconds since
/// the Unix epoch.
pub(crate) fn ts_fields(micros: i64) -> (i32, u32, u32, u32, u32, u32, u32) {
    let unix_days = micros.div_euclid(MICROS_PER_DAY);
    let rem = micros.rem_euclid(MICROS_PER_DAY);
    let (y, m, d) = civil_from_days(unix_days);
    let hour = (rem / MICROS_PER_HOUR) as u32;
    let minute = ((rem % MICROS_PER_HOUR) / MICROS_PER_MINUTE) as u32;
    let second = ((rem % MICROS_PER_MINUTE) / MICROS_PER_SECOND) as u32;
    let us = (rem % MICROS_PER_SECOND) as u32;
    (y as i32, m, d, hour, minute, second, us)
}

/// Truncate to the start of the given calendar unit (calendar floor; for the
/// month/year units the day-of-month and time components are zeroed).
pub(crate) fn trunc_ts(micros: i64, unit: TruncUnit) -> i64 {
    match unit {
        TruncUnit::Second => micros.div_euclid(MICROS_PER_SECOND) * MICROS_PER_SECOND,
        TruncUnit::Minute => micros.div_euclid(MICROS_PER_MINUTE) * MICROS_PER_MINUTE,
        TruncUnit::Hour => micros.div_euclid(MICROS_PER_HOUR) * MICROS_PER_HOUR,
        TruncUnit::Day => micros.div_euclid(MICROS_PER_DAY) * MICROS_PER_DAY,
        TruncUnit::Month | TruncUnit::Year => {
            let (y, m, _, _, _, _, _) = ts_fields(micros);
            let (y, m) = if unit == TruncUnit::Year {
                (y, 1)
            } else {
                (y, m)
            };
            let serial = days_from_civil(y as i64, m as i64, 1) + DAYS_0001_TO_1970;
            date_serial_to_unix_micros(serial as i32, 0, 0, 0, 0)
        }
    }
}

/// Combine a date serial (days since 0001-01-01) with time components into
/// Unix-epoch microseconds.
fn date_serial_to_unix_micros(date: i32, hour: u32, minute: u32, second: u32, micros: u32) -> i64 {
    let unix_days = date as i64 - DAYS_0001_TO_1970;
    unix_days * MICROS_PER_DAY
        + hour as i64 * MICROS_PER_HOUR
        + minute as i64 * MICROS_PER_MINUTE
        + second as i64 * MICROS_PER_SECOND
        + micros as i64
}

/// Timestamp → Date 日期序列（截断到日；CAST(ts AS DATE) 与 Iteration 001
/// date() 共用）。
pub(crate) fn ts_to_date_serial(micros: i64) -> i32 {
    let day_floor = trunc_ts(micros, TruncUnit::Day);
    (day_floor.div_euclid(MICROS_PER_DAY) + DAYS_0001_TO_1970) as i32
}

/// Date 日期序列 → Timestamp（当日零点扩展；CAST(d AS TIMESTAMP) 共用）。
pub(crate) fn date_serial_to_ts(date: i32) -> i64 {
    date_serial_to_unix_micros(date, 0, 0, 0, 0)
}

/// MS13 T4（决策 2）：日期族目标列的写入边界强制解析。目标列为 Date/
/// Timestamp 时值 SHALL 为同族值、Null，或可解析的 String（强制解析为对应
/// 类型值，与类型字面量等价）；其余类型值返回 `InvalidDateTime`。非日期族
/// 目标列原样放行。
pub(crate) fn coerce_datetime_write(
    value: &Value,
    col_type: &crate::storage::page_format::ColumnType,
) -> std::result::Result<Value, crate::storage::StorageError> {
    use crate::storage::page_format::ColumnType as CT;
    use crate::storage::StorageError;

    // 非日期族目标列原样放行
    let expected: &'static str = match col_type {
        CT::Date => "DATE",
        CT::Timestamp => "TIMESTAMP",
        _ => return Ok(value.clone()),
    };
    match (col_type, value) {
        // Null 放行（既有无键/NULL 语义照常）
        (_, Value::Null) => Ok(Value::Null),
        // 同族值原样
        (CT::Date, v @ Value::Date(_)) | (CT::Timestamp, v @ Value::Timestamp(_)) => Ok(v.clone()),
        // String 强制解析为对应类型值（解析失败含原值与期望类型名）
        (CT::Date, Value::String(s)) => {
            parse_date(s)
                .map(Value::Date)
                .ok_or(StorageError::InvalidDateTime {
                    value: s.clone(),
                    expected,
                })
        }
        (CT::Timestamp, Value::String(s)) => {
            parse_timestamp(s)
                .map(Value::Timestamp)
                .ok_or(StorageError::InvalidDateTime {
                    value: s.clone(),
                    expected,
                })
        }
        // 异族值显式拒绝（Date 值写入 Timestamp 列不跨族转换）
        _ => Err(StorageError::InvalidDateTime {
            value: value.to_string(),
            expected,
        }),
    }
}

/// Date 的日历截断（date_trunc Date 面：仅 year/month/day，调用方保证）。
pub(crate) fn trunc_date(days: i32, unit: TruncUnit) -> i32 {
    match unit {
        TruncUnit::Day => days,
        TruncUnit::Month | TruncUnit::Year => {
            let (y, m, _) = date_fields(days);
            let m = if unit == TruncUnit::Year { 1 } else { m };
            (days_from_civil(y as i64, m as i64, 1) + DAYS_0001_TO_1970) as i32
        }
        // 时间单位对 Date 无日历语义，调用方（date_trunc）先行类型拒绝；
        // 保持全等便于独立测试。
        _ => days,
    }
}

/// 展开后的日期时刻字段：`(y, mo, d, h, mi, s, us, micros_since_epoch)`。
type ExpandedFields = (i32, u32, u32, u32, u32, u32, u32, i64);

/// datediff（DA8 截断）：`b − a` 的整单位数，朝零截断（非四舍五入）。
/// day/hour/minute/second 为微秒差整除；month/year 为日历差——
/// `(y2−y1)×12+(m2−m1)`，(日,时,分,秒,微秒) 余量不足整月时向零收一
///（PostgreSQL age 语义方向；正负两方向对称，spec R3「绝对值按单位
/// 截断」）。a/b 同族由本函数先验；Date 视为零点时刻。
pub(crate) fn datediff(unit: TruncUnit, a: &Value, b: &Value) -> Result<i64, String> {
    let expand = |v: &Value| -> Result<ExpandedFields, String> {
        match v {
            Value::Date(d) => {
                let (y, m, dd) = date_fields(*d);
                Ok((y, m, dd, 0, 0, 0, 0, date_serial_to_ts(*d)))
            }
            Value::Timestamp(t) => {
                let f = ts_fields(*t);
                Ok((f.0, f.1, f.2, f.3, f.4, f.5, f.6, *t))
            }
            _ => Err("Type mismatch".to_string()),
        }
    };
    let same_family = matches!(
        (a, b),
        (Value::Date(_), Value::Date(_)) | (Value::Timestamp(_), Value::Timestamp(_))
    );
    if !same_family {
        return Err("Type mismatch".to_string());
    }
    let (ay, am, ad, ah, ami, as_, aus, a_micros) = expand(a)?;
    let (by, bm, bd, bh, bmi, bs, bus, b_micros) = expand(b)?;
    match unit {
        TruncUnit::Day => Ok((b_micros - a_micros) / MICROS_PER_DAY),
        TruncUnit::Hour => Ok((b_micros - a_micros) / MICROS_PER_HOUR),
        TruncUnit::Minute => Ok((b_micros - a_micros) / MICROS_PER_MINUTE),
        TruncUnit::Second => Ok((b_micros - a_micros) / MICROS_PER_SECOND),
        TruncUnit::Month | TruncUnit::Year => {
            let mut months = (by as i64 - ay as i64) * 12 + (bm as i64 - am as i64);
            let a_tail = (ad, ah, ami, as_, aus);
            let b_tail = (bd, bh, bmi, bs, bus);
            if months > 0 && b_tail < a_tail {
                months -= 1;
            } else if months < 0 && b_tail > a_tail {
                months += 1;
            }
            if unit == TruncUnit::Year {
                Ok(months / 12)
            } else {
                Ok(months)
            }
        }
    }
}

// ---- INTERVAL 表达式算术（MS13 T7，design D11/DA8） ----

/// 解析后的表达式内部区间值：月部分（year/month 锚定算术）与微秒部分
///（day/hour/minute/second 线性算术）。非 Value 变体——不可存储、不可
/// 比较、不入索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntervalParts {
    pub months: i32,
    pub micros: i64,
}

/// INTERVAL 字符串形态 `'N unit'`：单段 `<n> <unit>`，单位大小写不敏感、
/// 接受单复数，n 允许符号。多段（如 `'1 day 2 hours'`）、缺单位、未知
/// 单位、越界 → Err 点名。
pub(crate) fn parse_interval_string(s: &str) -> Result<IntervalParts, String> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid INTERVAL literal '{s}': expected a single '<n> <unit>' pair (multi-field intervals are not supported)"
        ));
    }
    let n: i64 = parts[0]
        .parse::<i64>()
        .map_err(|_| format!("invalid INTERVAL count '{}' in '{s}'", parts[0]))?;
    let unit = parts[1].to_ascii_lowercase();
    interval_parts_from_unit(&unit, n).ok_or_else(|| {
        format!(
            "unknown INTERVAL unit '{}' (supported: year, month, day, hour, minute, second)",
            parts[1]
        )
    })
}

/// 数值 + 单位字段形态（`INTERVAL 1 DAY`）的折算；单位以 DateTimeField
/// 已归一（调用方映射），越界 → None。
pub(crate) fn interval_parts_from_unit(unit: &str, n: i64) -> Option<IntervalParts> {
    let months = |m: i64| {
        i32::try_from(m).ok().map(|m| IntervalParts {
            months: m,
            micros: 0,
        })
    };
    let micros = |per: i64| {
        n.checked_mul(per).map(|m| IntervalParts {
            months: 0,
            micros: m,
        })
    };
    match unit {
        "year" | "years" => months(n.checked_mul(12)?),
        "month" | "months" => months(n),
        "day" | "days" => micros(MICROS_PER_DAY),
        "hour" | "hours" => micros(MICROS_PER_HOUR),
        "minute" | "minutes" => micros(MICROS_PER_MINUTE),
        "second" | "seconds" => micros(MICROS_PER_SECOND),
        _ => None,
    }
}

/// 月份同日锚定加法（DA8）：目标月按 `y*12+(m-1)+months` 进位，日
/// `min(d, 目标月天数)` 溢出截月末（2024-01-31 + 1 month = 2024-02-29）。
/// months 用 i64 承载（Sub 节点在求值期取负）。
pub(crate) fn add_months_ymd(y: i32, m: u32, d: u32, months: i64) -> (i32, u32, u32) {
    let total = y as i64 * 12 + (m as i64 - 1) + months;
    let ny = total.div_euclid(12);
    let nm = total.rem_euclid(12) as u32 + 1;
    let nd = (d as i64).min(days_in_month(ny, nm) as i64) as u32;
    (ny as i32, nm, nd)
}

const MAX_DATE_SERIAL: i64 = days_from_civil(9999, 12, 31) + DAYS_0001_TO_1970;

/// Date ± INTERVAL：先月（同日锚定截月末）后微秒；结果为 Date——微秒
/// 算术在零点时刻上进行，跨日按日地板（Date 无日内时刻）。日历范围
/// 0001-01-01..9999-12-31 越界 → Err 点名。months/micros 已含方向
///（Sub 在节点求值期取负）。
pub(crate) fn add_interval_to_date(days: i32, months: i64, micros: i64) -> Result<i32, String> {
    let (y, m, d) = date_fields(days);
    let (ny, nm, nd) = add_months_ymd(y, m, d, months);
    let base_serial = days_from_civil(ny as i64, nm as i64, nd as i64) + DAYS_0001_TO_1970;
    let ts = base_serial * MICROS_PER_DAY + micros;
    let serial = ts.div_euclid(MICROS_PER_DAY);
    if !(0..=MAX_DATE_SERIAL).contains(&serial) {
        return Err(
            "date arithmetic result out of supported range (0001-01-01..9999-12-31)".to_string(),
        );
    }
    Ok(serial as i32)
}

/// Timestamp ± INTERVAL：先月（同日锚定截月末，保留日内时刻）后微秒；
/// 结果日历部分越界（0001..9999）→ Err 点名。
pub(crate) fn add_interval_to_ts(micros: i64, months: i64, delta: i64) -> Result<i64, String> {
    let (y, m, d, h, mi, s, us) = ts_fields(micros);
    let (ny, nm, nd) = add_months_ymd(y, m, d, months);
    let base_unix_days = days_from_civil(ny as i64, nm as i64, nd as i64);
    let result = base_unix_days * MICROS_PER_DAY
        + h as i64 * MICROS_PER_HOUR
        + mi as i64 * MICROS_PER_MINUTE
        + s as i64 * MICROS_PER_SECOND
        + us as i64
        + delta;
    let (ry, _, _) = civil_from_days(result.div_euclid(MICROS_PER_DAY));
    if !(1..=9999).contains(&ry) {
        return Err(
            "timestamp arithmetic result out of supported range (0001-01-01..9999-12-31)"
                .to_string(),
        );
    }
    Ok(result)
}

/// Fixed-width digit helpers; `None` on any non-digit byte.
fn digits4(b: &[u8]) -> Option<i64> {
    let mut v = 0i64;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + i64::from(c - b'0');
    }
    Some(v)
}

fn digits2(b: &[u8]) -> Option<u32> {
    if b.len() != 2 || !b[0].is_ascii_digit() || !b[1].is_ascii_digit() {
        return None;
    }
    Some(u32::from(b[0] - b'0') * 10 + u32::from(b[1] - b'0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse/format date ----

    #[test]
    fn parse_date_valid_and_roundtrip() {
        let d = parse_date("2024-01-15").unwrap();
        assert_eq!(format_date(d), "2024-01-15");
    }

    #[test]
    fn parse_date_rejects_malformed() {
        assert_eq!(parse_date("2024-1-15"), None); // 非 4-2-2 定宽
        assert_eq!(parse_date("2024/01/15"), None);
        assert_eq!(parse_date("2024-13-01"), None); // month 13
        assert_eq!(parse_date("2024-00-10"), None); // month 0
        assert_eq!(parse_date("2024-01-00"), None); // day 0
        assert_eq!(parse_date("2024-01-15 "), None); // 尾随空格
    }

    #[test]
    fn parse_date_leap_year_rules() {
        assert!(parse_date("2024-02-29").is_some()); // 闰年
        assert_eq!(parse_date("2023-02-29"), None); // 平年
        assert_eq!(parse_date("1900-02-29"), None); // 100 非闰
        assert!(parse_date("2000-02-29").is_some()); // 400 闰
    }

    #[test]
    fn parse_date_month_ends() {
        assert!(parse_date("2024-01-31").is_some());
        assert!(parse_date("2024-04-30").is_some());
        assert_eq!(parse_date("2024-04-31"), None);
        assert!(parse_date("2024-12-31").is_some());
    }

    #[test]
    fn parse_date_boundary_years() {
        assert_eq!(parse_date("0001-01-01"), Some(0)); // 基准日 = 0
        assert!(parse_date("9999-12-31").is_some());
        assert_eq!(parse_date("0000-12-31"), None); // 年 0 越界
    }

    #[test]
    fn date_serial_anchor_1970() {
        // 锚点：0001-01-01 → 1970-01-01 = 719162 天（函数互证，非手抄）
        let d1970 = parse_date("1970-01-01").unwrap();
        assert_eq!(d1970 as i64, DAYS_0001_TO_1970);
        assert_eq!(d1970, 719162);
    }

    // ---- timestamp parse/format ----

    #[test]
    fn parse_timestamp_space_and_t_separators() {
        let ts = parse_timestamp("2024-01-15 10:30:00").unwrap();
        assert_eq!(parse_timestamp("2024-01-15T10:30:00"), Some(ts));
    }

    #[test]
    fn parse_timestamp_pure_date_is_midnight() {
        let ts = parse_timestamp("2024-01-15").unwrap();
        assert_eq!(ts.rem_euclid(MICROS_PER_DAY), 0);
        let (y, mo, d, h, mi, s, us) = ts_fields(ts);
        assert_eq!((y, mo, d), (2024, 1, 15));
        assert_eq!((h, mi, s, us), (0, 0, 0, 0));
    }

    #[test]
    fn parse_timestamp_fraction_digits() {
        let base = parse_timestamp("2024-01-15 10:30:00").unwrap();
        assert_eq!(
            parse_timestamp("2024-01-15 10:30:00.1"),
            Some(base + 100_000)
        );
        assert_eq!(
            parse_timestamp("2024-01-15 10:30:00.000001"),
            Some(base + 1)
        );
        assert_eq!(
            parse_timestamp("2024-01-15 10:30:00.123456"),
            Some(base + 123_456)
        );
        assert_eq!(parse_timestamp("2024-01-15 10:30:00.1234567"), None); // f>6 拒绝
        assert_eq!(parse_timestamp("2024-01-15 10:30:00."), None); // 空分数
    }

    #[test]
    fn parse_timestamp_rejects_timezone_suffix() {
        assert_eq!(parse_timestamp("2024-01-15T10:30:00Z"), None);
        assert_eq!(parse_timestamp("2024-01-15 10:30:00+08:00"), None);
    }

    #[test]
    fn parse_timestamp_rejects_bad_components() {
        assert_eq!(parse_timestamp("2024-01-15 24:00:00"), None); // hour
        assert_eq!(parse_timestamp("2024-01-15 10:60:00"), None); // minute
        assert_eq!(parse_timestamp("2024-01-15 10:30:60"), None); // second
        assert_eq!(parse_timestamp("2024-02-30 10:30:00"), None); // date part
    }

    #[test]
    fn timestamp_format_roundtrip_with_and_without_fraction() {
        let ts = parse_timestamp("2024-01-15 10:30:00").unwrap();
        assert_eq!(format_timestamp(ts), "2024-01-15 10:30:00"); // 无分数不加点
        let ts2 = parse_timestamp("2024-01-15 10:30:00.123456").unwrap();
        assert_eq!(format_timestamp(ts2), "2024-01-15 10:30:00.123456");
        let ts3 = parse_timestamp("2024-01-15 10:30:00.5").unwrap();
        assert_eq!(format_timestamp(ts3), "2024-01-15 10:30:00.500000");
    }

    #[test]
    fn timestamp_parse_format_roundtrip_identity() {
        for s in [
            "0001-01-01 00:00:00",
            "9999-12-31 23:59:59.999999",
            "1970-01-01 00:00:00",
        ] {
            let ts = parse_timestamp(s).unwrap();
            assert_eq!(format_timestamp(ts), s);
        }
    }

    // ---- field extraction ----

    #[test]
    fn date_fields_extraction() {
        assert_eq!(date_fields(0), (1, 1, 1));
        let d = parse_date("2024-02-29").unwrap();
        assert_eq!(date_fields(d), (2024, 2, 29));
    }

    #[test]
    fn date_fields_negative_days() {
        // serial -1 = 0000-12-31（历法外推仍成立，解析面已拒年 0）
        assert_eq!(date_fields(-1), (0, 12, 31));
    }

    #[test]
    fn ts_fields_extraction() {
        let ts = parse_timestamp("2024-02-29 13:45:06.000007").unwrap();
        let (y, mo, d, h, mi, s, us) = ts_fields(ts);
        assert_eq!((y, mo, d), (2024, 2, 29));
        assert_eq!((h, mi, s, us), (13, 45, 6, 7));
    }

    // ---- truncation ----

    #[test]
    fn trunc_ts_sub_day_units() {
        let ts = parse_timestamp("2024-01-15 10:30:45.123456").unwrap();
        let sec = parse_timestamp("2024-01-15 10:30:45").unwrap();
        let min = parse_timestamp("2024-01-15 10:30:00").unwrap();
        let hr = parse_timestamp("2024-01-15 10:00:00").unwrap();
        let day = parse_timestamp("2024-01-15").unwrap();
        assert_eq!(trunc_ts(ts, TruncUnit::Second), sec);
        assert_eq!(trunc_ts(ts, TruncUnit::Minute), min);
        assert_eq!(trunc_ts(ts, TruncUnit::Hour), hr);
        assert_eq!(trunc_ts(ts, TruncUnit::Day), day);
    }

    #[test]
    fn trunc_ts_month_and_year() {
        let ts = parse_timestamp("2024-03-15 10:30:45.123456").unwrap();
        assert_eq!(
            trunc_ts(ts, TruncUnit::Month),
            parse_timestamp("2024-03-01").unwrap()
        );
        assert_eq!(
            trunc_ts(ts, TruncUnit::Year),
            parse_timestamp("2024-01-01").unwrap()
        );
    }

    #[test]
    fn trunc_ts_negative_timestamp_floors() {
        // 1969-12-31 23:59:59.5 → 秒截断到 23:59:59；日截断到 1969-12-31
        let ts = parse_timestamp("1969-12-31 23:59:59.500000").unwrap();
        assert_eq!(
            trunc_ts(ts, TruncUnit::Second),
            parse_timestamp("1969-12-31 23:59:59").unwrap()
        );
        assert_eq!(
            trunc_ts(ts, TruncUnit::Day),
            parse_timestamp("1969-12-31").unwrap()
        );
    }

    #[test]
    fn trunc_ts_year_boundary_negative() {
        // 1969-07-20 → 月截断到 1969-07-01；年截断到 1969-01-01（负 epoch 区域）
        let ts = parse_timestamp("1969-07-20 20:17:40").unwrap();
        assert_eq!(
            trunc_ts(ts, TruncUnit::Month),
            parse_timestamp("1969-07-01").unwrap()
        );
        assert_eq!(
            trunc_ts(ts, TruncUnit::Year),
            parse_timestamp("1969-01-01").unwrap()
        );
    }

    // ---- INTERVAL 解析与锚定算术（T7） ----

    #[test]
    fn parse_interval_string_forms() {
        let p = parse_interval_string("1 day").unwrap();
        assert_eq!(p.micros, MICROS_PER_DAY);
        // 复数单位 + 大小写不敏感 + 负数
        let p = parse_interval_string("-90 Minutes").unwrap();
        assert_eq!(p.micros, -90 * MICROS_PER_MINUTE);
        // 月与年（年折 12 月）
        assert_eq!(parse_interval_string("2 month").unwrap().months, 2);
        assert_eq!(parse_interval_string("1 year").unwrap().months, 12);
        // 拒绝面：多段 / 缺单位 / 未知单位 / 非数字
        assert!(parse_interval_string("1 day 2 hours").is_err());
        assert!(parse_interval_string("1").is_err());
        assert!(parse_interval_string("1 week").is_err());
        assert!(parse_interval_string("x day").is_err());
    }

    #[test]
    fn interval_parts_from_unit_overflow_rejected() {
        // 巨值不 panic（checked 换算）
        assert!(interval_parts_from_unit("day", i64::MAX).is_none());
        assert!(interval_parts_from_unit("year", i64::MAX).is_none());
    }

    #[test]
    fn add_months_ymd_anchor_and_carry() {
        // 同日锚定 + 闰年截月末
        assert_eq!(add_months_ymd(2024, 1, 31, 1), (2024, 2, 29));
        // 平年截月末
        assert_eq!(add_months_ymd(2023, 1, 31, 1), (2023, 2, 28));
        // 年进位
        assert_eq!(add_months_ymd(2024, 12, 15, 1), (2025, 1, 15));
        // 负区间回退进位 + 锚定
        assert_eq!(add_months_ymd(2024, 3, 31, -1), (2024, 2, 29));
        assert_eq!(add_months_ymd(2024, 1, 15, -1), (2023, 12, 15));
        // 闰日 + 1 年锚定截月末
        assert_eq!(add_months_ymd(2024, 2, 29, 12), (2025, 2, 28));
    }

    #[test]
    fn add_interval_to_date_month_then_micros() {
        let d = parse_date("2024-01-31").unwrap();
        // 1 month → 2024-02-29；再加 1 day → 2024-03-01（锚定后线性）
        let m1 = add_interval_to_date(d, 1, 0).unwrap();
        assert_eq!(format_date(m1), "2024-02-29");
        let plus_day = add_interval_to_date(m1, 0, MICROS_PER_DAY).unwrap();
        assert_eq!(format_date(plus_day), "2024-03-01");
        // 负微秒跨日地板（-1 微秒回到前一日）
        let minus_us = add_interval_to_date(m1, 0, -1).unwrap();
        assert_eq!(format_date(minus_us), "2024-02-28");
        // 越界拒绝
        let max = parse_date("9999-12-31").unwrap();
        assert!(add_interval_to_date(max, 0, MICROS_PER_DAY).is_err());
        let min = parse_date("0001-01-01").unwrap();
        assert!(add_interval_to_date(min, 0, -1).is_err());
    }

    #[test]
    fn add_interval_to_ts_preserves_time_of_day() {
        let ts = parse_timestamp("2024-01-31 10:30:00").unwrap();
        let r = add_interval_to_ts(ts, 1, 0).unwrap();
        assert_eq!(format_timestamp(r), "2024-02-29 10:30:00");
        // 微秒部分
        let r = add_interval_to_ts(ts, 0, -90 * MICROS_PER_MINUTE).unwrap();
        assert_eq!(format_timestamp(r), "2024-01-31 09:00:00");
    }

    // ---- 写入边界强制解析（T4，决策 2） ----

    #[test]
    fn coerce_datetime_write_matrix() {
        use crate::storage::page_format::ColumnType as CT;

        // 非日期族列原样放行（含异形值）
        assert_eq!(
            coerce_datetime_write(&Value::String("x".into()), &CT::Int).unwrap(),
            Value::String("x".into())
        );
        // 同族值与 Null 原样
        assert_eq!(
            coerce_datetime_write(&Value::Date(10), &CT::Date).unwrap(),
            Value::Date(10)
        );
        assert_eq!(
            coerce_datetime_write(&Value::Null, &CT::Timestamp).unwrap(),
            Value::Null
        );
        // String 强制解析为对应类型
        assert_eq!(
            coerce_datetime_write(&Value::String("2024-01-15".into()), &CT::Date).unwrap(),
            Value::Date(parse_date("2024-01-15").unwrap())
        );
        assert_eq!(
            coerce_datetime_write(&Value::String("2024-01-15T10:30:00".into()), &CT::Timestamp)
                .unwrap(),
            Value::Timestamp(parse_timestamp("2024-01-15 10:30:00").unwrap())
        );
        // 解析失败 → InvalidDateTime（含原值与期望类型名）
        let err = coerce_datetime_write(&Value::String("2023-02-29".into()), &CT::Date)
            .unwrap_err()
            .to_string();
        assert!(err.contains("DATE") && err.contains("2023-02-29"), "{err}");
        // 异族值 → InvalidDateTime（Date 值写入 Timestamp 列不跨族转换）
        let err = coerce_datetime_write(&Value::Date(1), &CT::Timestamp).unwrap_err();
        assert!(err.to_string().contains("TIMESTAMP"), "{err}");
        // Int 值写入 Date 列 → InvalidDateTime
        assert!(coerce_datetime_write(&Value::Int(5), &CT::Date).is_err());
    }
}
