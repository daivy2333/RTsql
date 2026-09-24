//! Scalar function registry and evaluation (MS11-T03, MS13 T6).
//!
//! Single point of truth for the scalar function library: the metadata table
//! drives planner-side validation ([`is_scalar_function`] /
//! [`check_scalar_function`]) and executor-side dispatch in the same module,
//! so the plan-time name/arity contract and the runtime dispatch cannot
//! drift. Registry holds the string six, math four (MS11-T03) and the
//! datetime family ten (MS13 T6).

use crate::executor::datetime::{datediff, parse_trunc_unit, trunc_date, trunc_ts, TruncUnit};
use crate::executor::{Expression, ExpressionRef, Value, ValueError, ValueRef};

/// Function metadata: canonical uppercase name, inclusive arity range.
type ScalarFnMeta = (&'static str, usize, usize);

/// Registered scalar functions (MS11-T03: string six + math four; MS13 T6:
/// datetime family ten). Names are matched case-insensitively by canonicalizing
/// to uppercase at plan time. Aggregate names (COUNT/SUM/AVG/MIN/MAX) are
/// intentionally absent: they route through the aggregate path and keep their
/// existing value position rejections.
const REGISTRY: &[ScalarFnMeta] = &[
    ("UPPER", 1, 1),
    ("LOWER", 1, 1),
    ("LENGTH", 1, 1),
    ("SUBSTR", 2, 3),
    ("REPLACE", 3, 3),
    ("TRIM", 1, 1),
    ("ABS", 1, 1),
    ("ROUND", 1, 2),
    ("FLOOR", 1, 1),
    ("CEIL", 1, 1),
    // MS13 T6 datetime family (design D10). NOW is zero-arg: `now` is the
    // documented carve-out to the zero-arg rejection in the spec.
    ("NOW", 0, 0),
    ("DATE", 1, 1),
    ("YEAR", 1, 1),
    ("MONTH", 1, 1),
    ("DAY", 1, 1),
    ("HOUR", 1, 1),
    ("MINUTE", 1, 1),
    ("SECOND", 1, 1),
    ("DATE_TRUNC", 2, 2),
    ("DATEDIFF", 3, 3),
];

/// Whether `name` (canonical uppercase) is a registered scalar function.
pub fn is_scalar_function(name: &str) -> bool {
    REGISTRY.iter().any(|(n, _, _)| *n == name)
}

/// Plan-time arity check for a registered function. `Err` carries the named
/// message (function name + expected count); the planner wraps it into
/// `PlanError::ParseError`. Unregistered names must not reach this entry —
/// the planner keeps their existing `UnsupportedExpression` rejection.
pub fn check_scalar_function(name: &str, argc: usize) -> Result<(), String> {
    let (_, min, max) = REGISTRY
        .iter()
        .find(|(n, _, _)| *n == name)
        .ok_or_else(|| format!("Unknown scalar function '{}'", name))?;
    if argc >= *min && argc <= *max {
        return Ok(());
    }
    let expected = if min == max {
        min.to_string()
    } else {
        format!("{} or {}", min, max)
    };
    Err(format!(
        "Scalar function '{}' expects {} argument(s), got {}",
        name, expected, argc
    ))
}

/// Scalar function call expression (MS11-T03, design D1): one node for the
/// whole library; evaluation dispatches on the canonical uppercase [`name`].
/// The planner validates name, arity and the rejected AST forms (OVER /
/// DISTINCT / FILTER / named / wildcard args) against this module's registry
/// before building the node.
#[derive(Debug)]
pub struct FunctionExpression {
    /// Canonical uppercase registry name (dispatch key).
    pub name: String,
    pub args: Vec<ExpressionRef>,
}

impl FunctionExpression {
    /// Design D3 evaluation order: all arguments evaluate first (errors
    /// propagate before NULL handling) → any NULL argument yields NULL
    /// without type checks → otherwise dispatch on non-NULL values.
    fn eval_owned(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let mut vals = Vec::with_capacity(self.args.len());
        for arg in &self.args {
            vals.push(arg.evaluate(row)?);
        }
        if vals.iter().any(Value::is_null) {
            return Ok(Value::Null);
        }
        eval_scalar(&self.name, &vals).map_err(Box::<dyn std::error::Error + Send + Sync>::from)
    }
}

impl Expression for FunctionExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.eval_owned(row)
    }

    fn evaluate_ref<'a>(
        &'a self,
        row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        // Same shape as CASE/COALESCE/CAST (MS11-T01): materialize and reuse
        // the owned path. Only Copy variants can come back as a borrowed
        // view — computed String results have no backing storage to borrow.
        let owned: Vec<Value> = row.iter().map(ValueRef::to_value).collect();
        match self.eval_owned(&owned)? {
            Value::Int(n) => Ok(ValueRef::Int(n)),
            Value::Float(f) => Ok(ValueRef::Float(f)),
            Value::Bool(b) => Ok(ValueRef::Bool(b)),
            Value::Null => Ok(ValueRef::Null),
            // MS13: 日期族为 Copy 变体，直接回借
            Value::Date(d) => Ok(ValueRef::Date(d)),
            Value::Timestamp(t) => Ok(ValueRef::Timestamp(t)),
            Value::String(_) => Err(
                "Scalar function string results are not available on the zero-copy evaluate_ref path"
                    .into(),
            ),
        }
    }

    fn set_parameter_value(&self, param_name: &str, value: &Value) -> bool {
        self.args.iter().fold(false, |acc, a| {
            acc | a.set_parameter_value(param_name, value)
        })
    }
}

/// First argument as `&str`. NULL is already filtered upstream, so any
/// non-String value is a runtime type error (strict typing, no implicit
/// conversion — CAST is the explicit channel).
fn string_arg(args: &[Value], index: usize) -> Result<&str, String> {
    match &args[index] {
        Value::String(s) => Ok(s.as_str()),
        _ => Err(ValueError::TypeMismatch.to_string()),
    }
}

/// Integer argument; strict typing (Int only, same rationale as `string_arg`).
fn int_arg(args: &[Value], index: usize) -> Result<i64, String> {
    match &args[index] {
        Value::Int(n) => Ok(*n),
        _ => Err(ValueError::TypeMismatch.to_string()),
    }
}

/// Numeric argument as f64. Math functions accept Int or Float strictly
/// (spec R3); other types are a runtime type error, no string parsing.
fn float_arg(args: &[Value], index: usize) -> Result<f64, String> {
    match &args[index] {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(f) => Ok(*f),
        _ => Err(ValueError::TypeMismatch.to_string()),
    }
}

// ---- MS13 T6 datetime argument helpers (design D10, DA9) ----

/// `(year, month, day)` of a Date/Timestamp argument (strict: no coercion
/// from String/Int — CAST or typed literals are the explicit channels).
fn ymd_arg(args: &[Value], index: usize) -> Result<(i32, u32, u32), String> {
    match &args[index] {
        Value::Date(d) => Ok(crate::executor::datetime::date_fields(*d)),
        Value::Timestamp(t) => {
            let f = crate::executor::datetime::ts_fields(*t);
            Ok((f.0, f.1, f.2))
        }
        _ => Err(ValueError::TypeMismatch.to_string()),
    }
}

/// `(hour, minute, second)` of a Timestamp argument; a Date counts as
/// midnight (DA9).
fn hms_arg(args: &[Value], index: usize) -> Result<(u32, u32, u32), String> {
    match &args[index] {
        Value::Timestamp(t) => {
            let f = crate::executor::datetime::ts_fields(*t);
            Ok((f.3, f.4, f.5))
        }
        Value::Date(_) => Ok((0, 0, 0)),
        _ => Err(ValueError::TypeMismatch.to_string()),
    }
}

/// Date serial of a Date/Timestamp argument: Date identity, Timestamp
/// truncated to the day (`date(x)` semantics).
fn date_val_arg(args: &[Value], index: usize) -> Result<i32, String> {
    match &args[index] {
        Value::Date(d) => Ok(*d),
        Value::Timestamp(t) => Ok(crate::executor::datetime::ts_to_date_serial(*t)),
        _ => Err(ValueError::TypeMismatch.to_string()),
    }
}

/// SQLite-aligned substring over Unicode scalar values (1-based positions).
/// `start < 0` counts from the end (`substr('abc',-2)` = 'bc'); `start = 0`
/// occupies one phantom position before the first character
/// (`substr('abc',0,2)` = 'a'); a negative `len` takes `|len|` characters
/// *preceding* the start position (`substr('abc',2,-1)` = 'a'); an omitted
/// `len` (or one reaching past the tail) takes through the end of the string.
fn substr_chars(s: &str, start: i64, len: Option<i64>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    let pos = if start < 0 { n + start + 1 } else { start };
    match len {
        None => {
            let from = (pos - 1).max(0);
            if from >= n {
                String::new()
            } else {
                chars[from as usize..].iter().collect()
            }
        }
        Some(len) if len >= 0 => {
            // Phantom positions before 1 still consume the length budget.
            let phantom = if pos < 1 { 1 - pos } else { 0 };
            let from = (pos - 1).max(0) as usize;
            let take = (len - phantom).max(0) as usize;
            chars.iter().skip(from).take(take).collect()
        }
        Some(len) => {
            let end = ((pos - 1).max(0)).min(n) as usize;
            let begin = end.saturating_sub((-len) as usize);
            chars[begin..end].iter().collect()
        }
    }
}

fn eval_scalar(name: &str, args: &[Value]) -> Result<Value, String> {
    match name {
        "UPPER" => Ok(Value::String(string_arg(args, 0)?.to_ascii_uppercase())),
        "LOWER" => Ok(Value::String(string_arg(args, 0)?.to_ascii_lowercase())),
        "LENGTH" => Ok(Value::Int(string_arg(args, 0)?.chars().count() as i64)),
        "SUBSTR" => {
            let len = match args.get(2) {
                Some(_) => Some(int_arg(args, 2)?),
                None => None,
            };
            Ok(Value::String(substr_chars(
                string_arg(args, 0)?,
                int_arg(args, 1)?,
                len,
            )))
        }
        "REPLACE" => {
            let from = string_arg(args, 1)?;
            if from.is_empty() {
                // `str::replace` with an empty pattern inserts between every
                // char; SQLite returns the original string.
                return Ok(Value::String(string_arg(args, 0)?.to_string()));
            }
            Ok(Value::String(
                string_arg(args, 0)?.replace(from, string_arg(args, 2)?),
            ))
        }
        "TRIM" => Ok(Value::String(
            string_arg(args, 0)?.trim_matches(' ').to_string(),
        )),
        // Math four (spec R3): abs keeps the input type; round is
        // half-away-from-zero via `f64::round` and always returns Float
        // (digits Float truncates toward zero); floor/ceil return Float.
        "ABS" => match &args[0] {
            // I043（DA4）: i64::MIN 溢出显式运行时错误（不再 debug panic /
            // release 回绕）。
            Value::Int(n) => Ok(Value::Int(
                n.checked_abs()
                    .ok_or_else(|| format!("Integer overflow in abs({n})"))?,
            )),
            Value::Float(f) => Ok(Value::Float(f.abs())),
            _ => Err(ValueError::TypeMismatch.to_string()),
        },
        "ROUND" => {
            let x = float_arg(args, 0)?;
            let digits = match args.get(1) {
                Some(_) => match &args[1] {
                    Value::Int(n) => *n,
                    Value::Float(f) => *f as i64,
                    _ => return Err(ValueError::TypeMismatch.to_string()),
                },
                None => 0,
            };
            // I043（DA4）: |digits| 超出 f64 数量级表示范围时 SQLite 对齐
            // 饱和——正超界返回入参 Float 形态、负超界返回 0.0，杜绝
            // inf/NaN（`10f64.powi(±1000)` 分别溢出/下溢为 inf/0）。
            if digits > 308 {
                return Ok(Value::Float(x));
            }
            if digits < -308 {
                return Ok(Value::Float(0.0));
            }
            let factor = 10f64.powi(digits as i32);
            Ok(Value::Float((x * factor).round() / factor))
        }
        "FLOOR" => Ok(Value::Float(float_arg(args, 0)?.floor())),
        "CEIL" => Ok(Value::Float(float_arg(args, 0)?.ceil())),
        // MS13 T6 datetime family (design D10, DA9). NULL never reaches
        // here (D3 short-circuit upstream); strict typing throughout.
        "NOW" => {
            let micros = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_micros() as i64)
                .unwrap_or(0);
            Ok(Value::Timestamp(micros))
        }
        "DATE" => Ok(Value::Date(date_val_arg(args, 0)?)),
        "YEAR" => {
            let (y, _, _) = ymd_arg(args, 0)?;
            Ok(Value::Int(y as i64))
        }
        "MONTH" => {
            let (_, m, _) = ymd_arg(args, 0)?;
            Ok(Value::Int(m as i64))
        }
        "DAY" => {
            let (_, _, d) = ymd_arg(args, 0)?;
            Ok(Value::Int(d as i64))
        }
        "HOUR" => {
            let (h, _, _) = hms_arg(args, 0)?;
            Ok(Value::Int(h as i64))
        }
        "MINUTE" => {
            let (_, mi, _) = hms_arg(args, 0)?;
            Ok(Value::Int(mi as i64))
        }
        "SECOND" => {
            let (_, _, s) = hms_arg(args, 0)?;
            Ok(Value::Int(s as i64))
        }
        "DATE_TRUNC" => {
            let unit_str = string_arg(args, 0)?;
            let unit = parse_trunc_unit(unit_str)
                .ok_or_else(|| format!("unknown date_trunc unit '{unit_str}'"))?;
            match &args[1] {
                // Date 仅支持日历单位（DA9）；时间单位运行期类型错误。
                Value::Date(d) => match unit {
                    TruncUnit::Year | TruncUnit::Month | TruncUnit::Day => {
                        Ok(Value::Date(trunc_date(*d, unit)))
                    }
                    _ => Err(ValueError::TypeMismatch.to_string()),
                },
                Value::Timestamp(t) => Ok(Value::Timestamp(trunc_ts(*t, unit))),
                _ => Err(ValueError::TypeMismatch.to_string()),
            }
        }
        "DATEDIFF" => {
            let unit_str = string_arg(args, 0)?;
            let unit = parse_trunc_unit(unit_str)
                .ok_or_else(|| format!("unknown datediff unit '{unit_str}'"))?;
            Ok(Value::Int(datediff(unit, &args[1], &args[2])?))
        }
        // The registry and this match live in the same module on purpose: a
        // registered name without an arm is registry/impl drift and must fail
        // loudly instead of masquerading as a type error.
        other => unreachable!(
            "scalar function '{}' registered without an implementation",
            other
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{ConstantExpression, ParameterExpression};
    use std::sync::Arc;

    fn const_str(s: &str) -> ExpressionRef {
        Arc::new(ConstantExpression {
            value: Value::String(s.to_string()),
        })
    }

    fn fn_expr(name: &str, args: Vec<ExpressionRef>) -> FunctionExpression {
        FunctionExpression {
            name: name.to_string(),
            args,
        }
    }

    // ----- registry -----

    #[test]
    fn registry_membership() {
        for name in [
            "UPPER", "LOWER", "LENGTH", "SUBSTR", "REPLACE", "TRIM", "ABS", "ROUND", "FLOOR",
            "CEIL",
        ] {
            assert!(is_scalar_function(name));
        }
        // aggregates stay on the aggregate path; unknown names stay rejected
        for name in ["COUNT", "SUM", "AVG", "MIN", "MAX", "COALESCE", "NOPE"] {
            assert!(!is_scalar_function(name));
        }
    }

    #[test]
    fn registry_arity_checks() {
        assert!(check_scalar_function("UPPER", 1).is_ok());
        assert!(check_scalar_function("UPPER", 0).is_err());
        assert!(check_scalar_function("UPPER", 2).is_err());
        assert!(check_scalar_function("SUBSTR", 2).is_ok());
        assert!(check_scalar_function("SUBSTR", 3).is_ok());
        assert!(check_scalar_function("SUBSTR", 1).is_err());
        assert!(check_scalar_function("SUBSTR", 4).is_err());
        // named message: function name + expectation
        let msg = check_scalar_function("SUBSTR", 1).unwrap_err();
        assert!(msg.contains("SUBSTR"), "{msg}");
        assert!(msg.contains("argument"), "{msg}");
    }

    // ----- upper / lower -----

    #[test]
    fn upper_lower_ascii() {
        let upper = fn_expr("UPPER", vec![const_str("AbC")]);
        assert_eq!(upper.evaluate(&[]).unwrap(), Value::String("ABC".into()));
        let lower = fn_expr("LOWER", vec![const_str("AbC")]);
        assert_eq!(lower.evaluate(&[]).unwrap(), Value::String("abc".into()));
    }

    #[test]
    fn upper_type_mismatch_on_int() {
        let upper = fn_expr(
            "UPPER",
            vec![Arc::new(ConstantExpression {
                value: Value::Int(123),
            })],
        );
        assert!(upper.evaluate(&[]).is_err());
    }

    // ----- NULL semantics (D3) -----

    #[test]
    fn null_argument_yields_null_without_type_check() {
        let upper = fn_expr(
            "UPPER",
            vec![Arc::new(ConstantExpression { value: Value::Null })],
        );
        assert_eq!(upper.evaluate(&[]).unwrap(), Value::Null);
    }

    #[test]
    fn argument_error_propagates_before_null_check() {
        use crate::executor::{CastExpression, CastType};
        // arg 0 ERRORS at evaluation (CAST 'abc' → Int parse failure), arg 1
        // is NULL: the evaluation error must not be swallowed by the NULL
        // short-circuit (D3 order — distinct from the type check, which the
        // NULL short-circuit legitimately skips).
        let upper = fn_expr(
            "UPPER",
            vec![
                Arc::new(CastExpression {
                    expr: const_str("abc"),
                    target: CastType::Int,
                }),
                Arc::new(ConstantExpression { value: Value::Null }),
            ],
        );
        assert!(upper.evaluate(&[]).is_err());
    }

    // ----- evaluate_ref / set_parameter_value (D5) -----

    #[test]
    fn evaluate_ref_string_result_errors() {
        let upper = fn_expr("UPPER", vec![const_str("abc")]);
        let row: Vec<ValueRef> = vec![];
        assert!(upper.evaluate_ref(&row).is_err());
    }

    #[test]
    fn set_parameter_value_recurses_args() {
        // D5: recursion into args — a missed recursion silently returns
        // false, so the boolean contract is the regression witness. (The
        // evaluated String result is not asserted here: String parameter
        // values come back as NULL through ParameterExpression::evaluate_ref,
        // an M36-era limitation pre-existing outside this module.)
        let upper = fn_expr(
            "UPPER",
            vec![Arc::new(ParameterExpression::new("t.x".to_string()))],
        );
        assert!(upper.set_parameter_value("t.x", &Value::String("abc".into())));
        assert!(!upper.set_parameter_value("t.y", &Value::Null));
    }

    // ----- length / substr / replace / trim (T2) -----

    fn const_int(n: i64) -> ExpressionRef {
        Arc::new(ConstantExpression {
            value: Value::Int(n),
        })
    }

    #[test]
    fn length_counts_chars_not_bytes() {
        let len = fn_expr("LENGTH", vec![const_str("abc")]);
        assert_eq!(len.evaluate(&[]).unwrap(), Value::Int(3));
        let len = fn_expr("LENGTH", vec![const_str("你好")]);
        assert_eq!(len.evaluate(&[]).unwrap(), Value::Int(2));
        // strict typing: non-string is a runtime type error, no coercion
        let len = fn_expr("LENGTH", vec![const_int(123)]);
        assert!(len.evaluate(&[]).is_err());
    }

    #[test]
    fn substr_sqlite_edges() {
        // spec R2/S4 five locked assertions (s = 'abcdef') + basics
        let cases: [(&str, i64, Option<i64>, &str); 7] = [
            ("abcdef", 2, None, "bcdef"),  // omitted len → to the end
            ("abcdef", 2, Some(3), "bcd"), // regular window
            ("abcdef", 0, Some(2), "a"),   // start=0: one phantom position
            ("abcdef", -2, None, "ef"),    // negative start counts from end
            ("abcdef", 3, Some(-1), "b"),  // negative len: chars preceding start
            ("abc", 2, None, "bc"),
            ("abc", 4, None, ""), // start past the end
        ];
        for (s, start, len, expected) in cases {
            let mut args = vec![const_str(s), const_int(start)];
            if let Some(l) = len {
                args.push(const_int(l));
            }
            let sub = fn_expr("SUBSTR", args);
            assert_eq!(
                sub.evaluate(&[]).unwrap(),
                Value::String(expected.into()),
                "substr({s:?}, {start}, {len:?})"
            );
        }
        // start/len are strictly Int
        let sub = fn_expr(
            "SUBSTR",
            vec![
                const_str("abc"),
                Arc::new(ConstantExpression {
                    value: Value::Float(1.5),
                }),
            ],
        );
        assert!(sub.evaluate(&[]).is_err());
    }

    #[test]
    fn replace_all_and_empty_from() {
        let rep = fn_expr(
            "REPLACE",
            vec![const_str("a-b-a"), const_str("-"), const_str("+")],
        );
        assert_eq!(rep.evaluate(&[]).unwrap(), Value::String("a+b+a".into()));
        // empty `from` returns the original string (SQLite semantics)
        let rep = fn_expr(
            "REPLACE",
            vec![const_str("abc"), const_str(""), const_str("-")],
        );
        assert_eq!(rep.evaluate(&[]).unwrap(), Value::String("abc".into()));
    }

    #[test]
    fn trim_strips_spaces_only() {
        let cases: [(&str, &str); 4] = [
            ("  x ", "x"),      // leading/trailing spaces stripped
            ("\tx\t", "\tx\t"), // TAB preserved
            (" x y ", "x y"),   // inner spaces preserved
            ("   ", ""),        // all-space → empty
        ];
        for (input, expected) in cases {
            let trim = fn_expr("TRIM", vec![const_str(input)]);
            assert_eq!(
                trim.evaluate(&[]).unwrap(),
                Value::String(expected.into()),
                "trim({input:?})"
            );
        }
    }

    // ----- abs / round / floor / ceil (T4) -----

    fn const_float(f: f64) -> ExpressionRef {
        Arc::new(ConstantExpression {
            value: Value::Float(f),
        })
    }

    #[test]
    fn math_arity_checks() {
        for name in ["ABS", "FLOOR", "CEIL"] {
            assert!(check_scalar_function(name, 1).is_ok());
            assert!(check_scalar_function(name, 2).is_err());
        }
        assert!(check_scalar_function("ROUND", 1).is_ok());
        assert!(check_scalar_function("ROUND", 2).is_ok());
        assert!(check_scalar_function("ROUND", 0).is_err());
        assert!(check_scalar_function("ROUND", 3).is_err());
    }

    #[test]
    fn abs_keeps_input_type() {
        let abs_i = fn_expr("ABS", vec![const_int(-5)]);
        assert_eq!(abs_i.evaluate(&[]).unwrap(), Value::Int(5));
        let abs_f = fn_expr("ABS", vec![const_float(-5.5)]);
        assert_eq!(abs_f.evaluate(&[]).unwrap(), Value::Float(5.5));
        // strict typing: only Int|Float, no string coercion
        let abs_s = fn_expr("ABS", vec![const_str("x")]);
        assert!(abs_s.evaluate(&[]).is_err());
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.14/3.14159 are spec-locked rounding values, not PI
    fn round_half_away_from_zero_with_digits() {
        // spec R3/S2 locked values; digits Float truncates toward zero
        let cases: [(f64, Option<f64>, f64); 6] = [
            (3.7, None, 4.0),
            (2.5, None, 3.0),
            (-2.5, None, -3.0),
            (3.14159, Some(2.0), 3.14),
            (123.4, Some(-1.0), 120.0),
            (2.5, Some(0.7), 3.0), // digits 0.7 → 0
        ];
        for (x, digits, expected) in cases {
            let mut args = vec![const_float(x)];
            if let Some(d) = digits {
                args.push(const_float(d));
            }
            let r = fn_expr("ROUND", args);
            assert_eq!(
                r.evaluate(&[]).unwrap(),
                Value::Float(expected),
                "round({x}, {digits:?})"
            );
        }
        // strict typing on both positions
        let r = fn_expr("ROUND", vec![const_str("x")]);
        assert!(r.evaluate(&[]).is_err());
        let r = fn_expr("ROUND", vec![const_float(1.0), const_str("x")]);
        assert!(r.evaluate(&[]).is_err());
    }

    #[test]
    fn round_accepts_int_forms() {
        // round always returns Float; Int input and Int digits both accepted
        let r = fn_expr("ROUND", vec![const_int(2)]);
        assert_eq!(r.evaluate(&[]).unwrap(), Value::Float(2.0));
        let r = fn_expr("ROUND", vec![const_float(3.7), const_int(0)]);
        assert_eq!(r.evaluate(&[]).unwrap(), Value::Float(4.0));
    }

    #[test]
    fn floor_ceil_return_float() {
        let cases: [(&str, f64, f64); 4] = [
            ("FLOOR", 3.7, 3.0),
            ("CEIL", 3.2, 4.0),
            ("FLOOR", -3.7, -4.0),
            ("CEIL", -3.7, -3.0),
        ];
        for (name, x, expected) in cases {
            let f = fn_expr(name, vec![const_float(x)]);
            assert_eq!(
                f.evaluate(&[]).unwrap(),
                Value::Float(expected),
                "{name}({x})"
            );
        }
        // Int input converts to a Float result
        let f = fn_expr("FLOOR", vec![const_int(3)]);
        assert_eq!(f.evaluate(&[]).unwrap(), Value::Float(3.0));
        // strict typing
        let f = fn_expr("CEIL", vec![const_str("x")]);
        assert!(f.evaluate(&[]).is_err());
    }

    #[test]
    fn math_null_argument_yields_null() {
        for name in ["ABS", "ROUND", "FLOOR", "CEIL"] {
            let f = fn_expr(
                name,
                vec![Arc::new(ConstantExpression { value: Value::Null })],
            );
            assert_eq!(f.evaluate(&[]).unwrap(), Value::Null, "{name}(NULL)");
        }
    }
}
