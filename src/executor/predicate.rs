//! Predicate and Expression traits for WHERE clause evaluation
//!
//! Task 8: WHERE clause expression evaluator

use crate::executor::{Value, ValueError, ValueRef};
use std::fmt::Debug;
use std::sync::Arc;

/// Three-valued logic result for predicate evaluation (MS11-T01).
///
/// SQL semantics: a predicate over a NULL operand is `Unknown`; row selection
/// folds `Unknown` to no-match via [`fold`], so observable behavior of the
/// former two-valued path is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ternary {
    True,
    False,
    Unknown,
}

/// Fold a ternary result into a row-selection bool: Unknown never matches.
pub fn fold(t: Ternary) -> bool {
    matches!(t, Ternary::True)
}

/// Predicate trait - evaluates a row against a boolean condition
pub trait Predicate: Send + Sync + Debug {
    /// Evaluate the predicate against a row
    /// Returns true if the row satisfies the predicate
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;

    /// MS11-T01: three-valued evaluation. Default maps the two-valued
    /// `evaluate` result; NULL-aware implementations override this.
    fn evaluate_ternary(
        &self,
        row: &[Value],
    ) -> Result<Ternary, Box<dyn std::error::Error + Send + Sync>> {
        self.evaluate(row)
            .map(|b| if b { Ternary::True } else { Ternary::False })
    }

    /// Recursively inject correlated parameter values into ParameterExpression nodes
    fn inject_parameters(&self, _params: &[(String, Value)]) {}
}

/// Reference to a Predicate (Arc<dyn Predicate>)
pub type PredicateRef = Arc<dyn Predicate>;

/// Expression trait - evaluates to a value
pub trait Expression: Send + Sync + Debug {
    /// Backward-compatible owned entry point. Default impl converts the row
    /// to `&[ValueRef]` via `as_value_ref` and calls `evaluate_ref`, then
    /// `to_value()`s the result. Implementations SHOULD override only
    /// `evaluate_ref`; the default `evaluate` will call it correctly.
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let row_ref: Vec<ValueRef> = row.iter().map(Value::as_value_ref).collect();
        self.evaluate_ref(&row_ref).map(|vr| vr.to_value())
    }

    /// M36: zero-copy entry point. Returns a `ValueRef<'a>` borrowing
    /// from `&'a self` or `&'a row` (both share lifetime `'a`).
    /// Implementations MUST NOT `.await` and MUST NOT
    /// recursively call `BufferPool` methods (deadlock risk).
    fn evaluate_ref<'a>(
        &'a self,
        row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>>;

    /// Try to set a parameter value by name. Returns true if matched and set.
    fn set_parameter_value(&self, _param_name: &str, _value: &Value) -> bool {
        false
    }
}

/// Reference to an Expression (Arc<dyn Expression>)
pub type ExpressionRef = Arc<dyn Expression>;

/// Comparison operators
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComparisonOp {
    /// Equal (=)
    Eq,
    /// Not equal (!=)
    Ne,
    /// Greater than (>)
    Gt,
    /// Less than (<)
    Lt,
    /// Greater than or equal (>=)
    Ge,
    /// Less than or equal (<=)
    Le,
}

/// Comparison predicate (e.g., id = 5, value > 10)
#[derive(Debug)]
pub struct ComparisonPredicate {
    pub left: ExpressionRef,
    pub op: ComparisonOp,
    pub right: ExpressionRef,
}

impl Predicate for ComparisonPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.evaluate_ternary(row).map(fold)
    }

    fn evaluate_ternary(
        &self,
        row: &[Value],
    ) -> Result<Ternary, Box<dyn std::error::Error + Send + Sync>> {
        let left_val = self.left.evaluate(row)?;
        let right_val = self.right.evaluate(row)?;

        // SQL three-valued semantics: a comparison with a NULL operand is
        // Unknown (row selection folds it to no-match, byte-identical to the
        // former `NULL -> Ok(false)` path).
        if left_val.is_null() || right_val.is_null() {
            return Ok(Ternary::Unknown);
        }

        // MS13 T5（决策 2）：日期族参与的比较严格同类型——任一侧为
        // Date/Timestamp 且两侧非同变体（含 Date×Timestamp 与日期×非日期族）
        // 时显式类型错误（datetime-type-system R4「比较严格」）。既有五类型
        // 互比的 equals-false 兜底不受影响（R7 零回归）。
        if matches!(left_val, Value::Date(_) | Value::Timestamp(_))
            || matches!(right_val, Value::Date(_) | Value::Timestamp(_))
        {
            let same_variant = matches!(
                (&left_val, &right_val),
                (Value::Date(_), Value::Date(_)) | (Value::Timestamp(_), Value::Timestamp(_))
            );
            if !same_variant {
                return Err(Box::new(ValueError::TypeMismatch));
            }
        }

        let result = match self.op {
            ComparisonOp::Eq => Ok(left_val.equals(&right_val)),
            ComparisonOp::Ne => Ok(!left_val.equals(&right_val)),
            ComparisonOp::Gt => left_val.gt(&right_val),
            ComparisonOp::Lt => left_val.lt(&right_val),
            ComparisonOp::Ge => left_val.ge(&right_val),
            ComparisonOp::Le => left_val.le(&right_val),
        };

        result
            .map(|b| if b { Ternary::True } else { Ternary::False })
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn inject_parameters(&self, params: &[(String, Value)]) {
        for (name, value) in params {
            self.left.set_parameter_value(name, value);
            self.right.set_parameter_value(name, value);
        }
    }
}

/// Logical operators
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogicalOp {
    /// AND
    And,
    /// OR
    Or,
}

/// Logical predicate (e.g., id > 10 AND value < 100)
#[derive(Debug)]
pub struct LogicalPredicate {
    pub left: PredicateRef,
    pub op: LogicalOp,
    pub right: PredicateRef,
}

impl Predicate for LogicalPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.evaluate_ternary(row).map(fold)
    }

    fn evaluate_ternary(
        &self,
        row: &[Value],
    ) -> Result<Ternary, Box<dyn std::error::Error + Send + Sync>> {
        match self.op {
            LogicalOp::And => {
                let left = self.left.evaluate_ternary(row)?;
                // False dominates AND: skip the right side exactly like the
                // former boolean short-circuit, preserving its observable
                // behavior and error propagation.
                if left == Ternary::False {
                    return Ok(Ternary::False);
                }
                let right = self.right.evaluate_ternary(row)?;
                Ok(match (left, right) {
                    (_, Ternary::False) => Ternary::False,
                    (Ternary::True, Ternary::True) => Ternary::True,
                    _ => Ternary::Unknown,
                })
            }
            LogicalOp::Or => {
                let left = self.left.evaluate_ternary(row)?;
                // True dominates OR: same short-circuit preservation as AND.
                if left == Ternary::True {
                    return Ok(Ternary::True);
                }
                let right = self.right.evaluate_ternary(row)?;
                Ok(match (left, right) {
                    (_, Ternary::True) => Ternary::True,
                    (Ternary::False, Ternary::False) => Ternary::False,
                    _ => Ternary::Unknown,
                })
            }
        }
    }

    fn inject_parameters(&self, params: &[(String, Value)]) {
        self.left.inject_parameters(params);
        self.right.inject_parameters(params);
    }
}

/// LIKE predicate: `%` matches any run of characters, `_` matches exactly one
/// (MS11-T01). Both operands are evaluated per row; negation is composed via
/// `NotPredicate` at the planner, so this type carries no `negated` field.
#[derive(Debug)]
pub struct LikePredicate {
    pub expr: ExpressionRef,
    pub pattern: ExpressionRef,
}

impl Predicate for LikePredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.evaluate_ternary(row).map(fold)
    }

    fn evaluate_ternary(
        &self,
        row: &[Value],
    ) -> Result<Ternary, Box<dyn std::error::Error + Send + Sync>> {
        let value = self.expr.evaluate(row)?;
        let pattern = self.pattern.evaluate(row)?;
        if value.is_null() || pattern.is_null() {
            return Ok(Ternary::Unknown);
        }
        let (value, pattern) = match (&value, &pattern) {
            (Value::String(v), Value::String(p)) => (v.as_str(), p.as_str()),
            _ => {
                return Err(
                    Box::new(ValueError::TypeMismatch) as Box<dyn std::error::Error + Send + Sync>
                )
            }
        };
        Ok(if like_match(value, pattern) {
            Ternary::True
        } else {
            Ternary::False
        })
    }

    fn inject_parameters(&self, params: &[(String, Value)]) {
        for (name, value) in params {
            self.expr.set_parameter_value(name, value);
            self.pattern.set_parameter_value(name, value);
        }
    }
}

/// SQL LIKE matching with `%` (any run of characters) and `_` (exactly one).
/// Greedy scan with backtracking on the last `%`; every other character in the
/// pattern matches literally (no escape processing at this layer).
fn like_match(value: &str, pattern: &str) -> bool {
    let v: Vec<char> = value.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    let (mut vi, mut pi) = (0usize, 0usize);
    let (mut star_v, mut star_p) = (None::<usize>, 0usize);
    while vi < v.len() {
        if pi < p.len() && (p[pi] == '_' || p[pi] == v[vi]) {
            vi += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == '%' {
            star_v = Some(vi);
            star_p = pi;
            pi += 1;
        } else if let Some(sv) = star_v {
            // Let the last `%` absorb one more character and retry.
            vi = sv + 1;
            star_v = Some(vi);
            pi = star_p + 1;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '%' {
        pi += 1;
    }
    pi == p.len()
}

/// IS NULL predicate: True only for NULL operands, never Unknown (MS11-T01).
#[derive(Debug)]
pub struct IsNullPredicate {
    pub expr: ExpressionRef,
}

impl Predicate for IsNullPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.evaluate_ternary(row).map(fold)
    }

    fn evaluate_ternary(
        &self,
        row: &[Value],
    ) -> Result<Ternary, Box<dyn std::error::Error + Send + Sync>> {
        let value = self.expr.evaluate(row)?;
        Ok(if value.is_null() {
            Ternary::True
        } else {
            Ternary::False
        })
    }

    fn inject_parameters(&self, params: &[(String, Value)]) {
        for (name, value) in params {
            self.expr.set_parameter_value(name, value);
        }
    }
}

/// Three-valued negation (MS11-T01): NOT True = False, NOT False = True,
/// NOT Unknown = Unknown. All `[NOT]` forms are planner-desugared to this.
#[derive(Debug)]
pub struct NotPredicate {
    pub inner: PredicateRef,
}

impl Predicate for NotPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.evaluate_ternary(row).map(fold)
    }

    fn evaluate_ternary(
        &self,
        row: &[Value],
    ) -> Result<Ternary, Box<dyn std::error::Error + Send + Sync>> {
        Ok(match self.inner.evaluate_ternary(row)? {
            Ternary::True => Ternary::False,
            Ternary::False => Ternary::True,
            Ternary::Unknown => Ternary::Unknown,
        })
    }

    fn inject_parameters(&self, params: &[(String, Value)]) {
        self.inner.inject_parameters(params);
    }
}

/// Column expression - evaluates to a column value from the row
#[derive(Debug)]
pub struct ColumnExpression {
    pub column_name: String,
    pub column_index: usize,
}

impl Expression for ColumnExpression {
    fn evaluate_ref<'a>(
        &'a self,
        row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        match row.get(self.column_index) {
            Some(v) => Ok(*v),
            None => Err(format!(
                "Column index {} out of bounds (row has {} columns)",
                self.column_index,
                row.len()
            )
            .into()),
        }
    }
}

/// Constant expression - evaluates to a constant value
#[derive(Debug)]
pub struct ConstantExpression {
    pub value: Value,
}

impl Expression for ConstantExpression {
    fn evaluate_ref<'a>(
        &'a self,
        _row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.value.as_value_ref())
    }
}

/// Parameter expression - a placeholder for correlated outer column value
/// The value is injected at execution time via set_parameter_value
pub struct ParameterExpression {
    pub param_name: String,
    value: std::sync::Mutex<Value>,
}

impl ParameterExpression {
    pub fn new(param_name: String) -> Self {
        Self {
            param_name,
            value: std::sync::Mutex::new(Value::Null),
        }
    }

    pub fn set_value(&self, value: Value) {
        *self.value.lock().unwrap() = value;
    }
}

impl Debug for ParameterExpression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterExpression")
            .field("param_name", &self.param_name)
            .finish()
    }
}

impl Expression for ParameterExpression {
    fn evaluate_ref<'a>(
        &'a self,
        _row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        // M36 T5: For Int/Float/Bool/Null, the value is Copy and can be returned directly.
        // For String, we cannot return a stable &'a str borrow from the MutexGuard
        // (it's a temporary), and M36 scope does not require zero-copy for parameter
        // String values (no production callers do this). Return Null for the String
        // case as a safe default; M37 may add Arc<str>-based zero-copy for this.
        let guard = self.value.lock().unwrap();
        Ok(match &*guard {
            Value::Int(n) => ValueRef::Int(*n),
            Value::Float(f) => ValueRef::Float(*f),
            Value::Bool(b) => ValueRef::Bool(*b),
            Value::Null => ValueRef::Null,
            // MS13: 日期族为 Copy 变体，直接回借
            Value::Date(d) => ValueRef::Date(*d),
            Value::Timestamp(t) => ValueRef::Timestamp(*t),
            Value::String(_) => ValueRef::Null, // M37 TODO: Arc<str> zero-copy
        })
    }

    fn set_parameter_value(&self, param_name: &str, value: &Value) -> bool {
        if self.param_name == param_name {
            *self.value.lock().unwrap() = value.clone();
            true
        } else {
            false
        }
    }
}

/// CAST target types (MS11-T01): strict four-family mapping; unknown types are
/// rejected at planning time by the planner's strict converter. MS13 T5 adds
/// the Date/Timestamp targets (datetime-type-system R5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CastType {
    Int,
    Float,
    String,
    Bool,
    Date,
    Timestamp,
}

/// CASE expression (MS11-T01). The searched form is built directly; the simple
/// form (`CASE operand WHEN v ...`) is planner-desugared into Eq comparisons,
/// so a NULL operand compares Unknown and never matches a WHEN branch.
#[derive(Debug)]
pub struct CaseExpression {
    pub whens: Vec<(PredicateRef, ExpressionRef)>,
    pub else_: Option<ExpressionRef>,
}

impl CaseExpression {
    fn eval_owned(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        for (cond, result) in &self.whens {
            if matches!(cond.evaluate_ternary(row)?, Ternary::True) {
                return result.evaluate(row);
            }
        }
        match &self.else_ {
            Some(e) => e.evaluate(row),
            None => Ok(Value::Null),
        }
    }
}

impl Expression for CaseExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.eval_owned(row)
    }

    fn evaluate_ref<'a>(
        &'a self,
        row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        // Conditions are Predicates over owned rows: materialize and reuse the
        // owned path. Only Copy variants can come back as a borrowed view —
        // computed String results have no backing storage to borrow from.
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
                "CASE string results are not available on the zero-copy evaluate_ref path".into(),
            ),
        }
    }

    fn set_parameter_value(&self, param_name: &str, value: &Value) -> bool {
        let mut matched = false;
        for (cond, result) in &self.whens {
            cond.inject_parameters(&[(param_name.to_string(), value.clone())]);
            matched |= result.set_parameter_value(param_name, value);
        }
        if let Some(e) = &self.else_ {
            matched |= e.set_parameter_value(param_name, value);
        }
        matched
    }
}

/// COALESCE expression (MS11-T01): first non-NULL argument, all-NULL → NULL.
#[derive(Debug)]
pub struct CoalesceExpression {
    pub args: Vec<ExpressionRef>,
}

impl Expression for CoalesceExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        for arg in &self.args {
            let v = arg.evaluate(row)?;
            if !v.is_null() {
                return Ok(v);
            }
        }
        Ok(Value::Null)
    }

    fn evaluate_ref<'a>(
        &'a self,
        row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        let owned: Vec<Value> = row.iter().map(ValueRef::to_value).collect();
        match self.evaluate(&owned)? {
            Value::Int(n) => Ok(ValueRef::Int(n)),
            Value::Float(f) => Ok(ValueRef::Float(f)),
            Value::Bool(b) => Ok(ValueRef::Bool(b)),
            Value::Null => Ok(ValueRef::Null),
            // MS13: 日期族为 Copy 变体，直接回借
            Value::Date(d) => Ok(ValueRef::Date(d)),
            Value::Timestamp(t) => Ok(ValueRef::Timestamp(t)),
            Value::String(_) => Err(
                "COALESCE string results are not available on the zero-copy evaluate_ref path"
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

/// CAST expression (MS11-T01). NULL short-circuits to NULL; conversions follow
/// the documented matrix: numeric↔string parsed/formatted by value, Float→Int
/// truncates toward zero with `as` saturation, Bool↔numeric rejected.
#[derive(Debug)]
pub struct CastExpression {
    pub expr: ExpressionRef,
    pub target: CastType,
}

impl CastExpression {
    fn cast_value(value: &Value, target: CastType) -> Result<Value, ValueError> {
        if value.is_null() {
            return Ok(Value::Null);
        }
        Ok(match (target, value) {
            // Identity
            (CastType::Int, Value::Int(n)) => Value::Int(*n),
            (CastType::Float, Value::Float(f)) => Value::Float(*f),
            (CastType::String, Value::String(s)) => Value::String(s.clone()),
            (CastType::Bool, Value::Bool(b)) => Value::Bool(*b),
            // Numeric ↔ numeric
            (CastType::Int, Value::Float(f)) => Value::Int(*f as i64),
            (CastType::Float, Value::Int(n)) => Value::Float(*n as f64),
            // String → numeric/bool: parse failure is a runtime type error
            (CastType::Int, Value::String(s)) => {
                Value::Int(s.parse::<i64>().map_err(|_| ValueError::TypeMismatch)?)
            }
            (CastType::Float, Value::String(s)) => {
                Value::Float(s.parse::<f64>().map_err(|_| ValueError::TypeMismatch)?)
            }
            (CastType::Bool, Value::String(s)) => {
                Value::Bool(s.parse::<bool>().map_err(|_| ValueError::TypeMismatch)?)
            }
            // Numeric/bool → string: value formatting (NOT `Value::Display`,
            // which quotes strings)
            (CastType::String, Value::Int(n)) => Value::String(n.to_string()),
            (CastType::String, Value::Float(f)) => Value::String(f.to_string()),
            (CastType::String, Value::Bool(b)) => Value::String(b.to_string()),
            // MS13 T5（D8）：日期族 CAST 矩阵——String 解析严格（失败显式
            // 错误）、格式化按 DA5、Timestamp→Date 截断到日、Date→Timestamp
            // 零点扩展；数值/Bool×日期族落入既有 `_` 拒绝。
            (CastType::Date, Value::Date(_)) => value.clone(),
            (CastType::Timestamp, Value::Timestamp(_)) => value.clone(),
            (CastType::Date, Value::String(s)) => {
                let d = crate::executor::datetime::parse_date(s).ok_or(ValueError::TypeMismatch)?;
                Value::Date(d)
            }
            (CastType::Timestamp, Value::String(s)) => {
                let t = crate::executor::datetime::parse_timestamp(s)
                    .ok_or(ValueError::TypeMismatch)?;
                Value::Timestamp(t)
            }
            (CastType::String, Value::Date(d)) => {
                Value::String(crate::executor::datetime::format_date(*d))
            }
            (CastType::String, Value::Timestamp(t)) => {
                Value::String(crate::executor::datetime::format_timestamp(*t))
            }
            (CastType::Date, Value::Timestamp(t)) => {
                Value::Date(crate::executor::datetime::ts_to_date_serial(*t))
            }
            (CastType::Timestamp, Value::Date(d)) => {
                Value::Timestamp(crate::executor::datetime::date_serial_to_ts(*d))
            }
            // Bool↔numeric and every other cross-family conversion is rejected
            _ => return Err(ValueError::TypeMismatch),
        })
    }
}

impl Expression for CastExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let value = self.expr.evaluate(row)?;
        Self::cast_value(&value, self.target)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn evaluate_ref<'a>(
        &'a self,
        row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        let owned: Vec<Value> = row.iter().map(ValueRef::to_value).collect();
        match self.evaluate(&owned)? {
            Value::Int(n) => Ok(ValueRef::Int(n)),
            Value::Float(f) => Ok(ValueRef::Float(f)),
            Value::Bool(b) => Ok(ValueRef::Bool(b)),
            Value::Null => Ok(ValueRef::Null),
            // MS13: 日期族为 Copy 变体，直接回借（DA8 矩阵扩展见 cast_value）
            Value::Date(d) => Ok(ValueRef::Date(d)),
            Value::Timestamp(t) => Ok(ValueRef::Timestamp(t)),
            Value::String(_) => Err(
                "CAST string results are not available on the zero-copy evaluate_ref path".into(),
            ),
        }
    }

    fn set_parameter_value(&self, param_name: &str, value: &Value) -> bool {
        self.expr.set_parameter_value(param_name, value)
    }
}

/// Arithmetic operator for [`BinaryArithExpression`] (MS13 T4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Arithmetic binary expression (MS13 T4): `+`/`-`/`*`/`/` over numeric
/// operands — the compilation path for WITH-FORM arithmetic projection items
/// (`SELECT id + 1 FROM t`, no-from-select R3) and WHERE arithmetic
/// comparison legs. INTERVAL legs are probed before this node and compile to
/// [`IntervalArithExpression`] (design D11), so this node never carries
/// intervals. NULL propagates to NULL; a non-numeric operand is a runtime
/// type error (strict typing); Int/Int division truncates toward zero and a
/// zero divisor yields NULL (the existing `Value::div` aggregate convention).
#[derive(Debug)]
pub struct BinaryArithExpression {
    pub left: ExpressionRef,
    pub op: ArithOp,
    pub right: ExpressionRef,
}

impl BinaryArithExpression {
    fn eval(l: &Value, op: ArithOp, r: &Value) -> Result<Value, ValueError> {
        if l.is_null() || r.is_null() {
            return Ok(Value::Null);
        }
        // 严格数值面：非数值操作数显式类型错误（四操作符统一语义）。
        if !matches!(l, Value::Int(_) | Value::Float(_))
            || !matches!(r, Value::Int(_) | Value::Float(_))
        {
            return Err(ValueError::TypeMismatch);
        }
        Ok(match op {
            // 复用既有聚合算术（Int/Int→Int 截断除、零除→Null、跨族提升 Float）
            ArithOp::Add => l.add(r),
            ArithOp::Div => l.div(r),
            ArithOp::Sub => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a - b),
                (Value::Float(a), Value::Float(b)) => Value::Float(a - b),
                (Value::Int(a), Value::Float(b)) => Value::Float(*a as f64 - b),
                (Value::Float(a), Value::Int(b)) => Value::Float(a - *b as f64),
                _ => unreachable!("numeric guard above"),
            },
            ArithOp::Mul => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a * b),
                (Value::Float(a), Value::Float(b)) => Value::Float(a * b),
                (Value::Int(a), Value::Float(b)) => Value::Float(*a as f64 * b),
                (Value::Float(a), Value::Int(b)) => Value::Float(a * *b as f64),
                _ => unreachable!("numeric guard above"),
            },
        })
    }
}

impl Expression for BinaryArithExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let l = self.left.evaluate(row)?;
        let r = self.right.evaluate(row)?;
        Self::eval(&l, self.op, &r)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn evaluate_ref<'a>(
        &'a self,
        row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        // Same shape as CASE/COALESCE/CAST: materialize and reuse the owned
        // path; Int/Float/Null results are Copy and come back as borrowed
        // views directly.
        let owned: Vec<Value> = row.iter().map(ValueRef::to_value).collect();
        match self.evaluate(&owned)? {
            Value::Int(n) => Ok(ValueRef::Int(n)),
            Value::Float(f) => Ok(ValueRef::Float(f)),
            Value::Null => Ok(ValueRef::Null),
            _ => Err("Arithmetic results are numeric; non-numeric here is a bug".into()),
        }
    }

    fn set_parameter_value(&self, param_name: &str, value: &Value) -> bool {
        self.left.set_parameter_value(param_name, value)
            | self.right.set_parameter_value(param_name, value)
    }
}

/// INTERVAL 算术表达式（MS13 T7，design D11）：`<date/timestamp> ± INTERVAL`。
/// 月份部分按同日锚定截月末（DA8）先应用，微秒部分线性加减，方向（Sub）
/// 在求值期取负；NULL 左操作数传播为 NULL；左操作数非 Date/Timestamp 为
/// 运行时类型错误（严格类型；`op` 由 planner 只构造 Add/Sub，其余在求值期
/// 拒绝为 bug 面）。结果日历范围 0001-01-01..9999-12-31 越界显式报错。
#[derive(Debug)]
pub struct IntervalArithExpression {
    pub left: ExpressionRef,
    pub op: ArithOp,
    pub interval: crate::executor::datetime::IntervalParts,
}

impl IntervalArithExpression {
    fn eval(&self, l: &Value) -> Result<Value, String> {
        if l.is_null() {
            return Ok(Value::Null);
        }
        let (months, micros) = match self.op {
            ArithOp::Add => (self.interval.months as i64, self.interval.micros),
            ArithOp::Sub => (-(self.interval.months as i64), -self.interval.micros),
            _ => return Err("INTERVAL arithmetic only supports + and -".to_string()),
        };
        match l {
            Value::Date(d) => Ok(Value::Date(
                crate::executor::datetime::add_interval_to_date(*d, months, micros)?,
            )),
            Value::Timestamp(t) => Ok(Value::Timestamp(
                crate::executor::datetime::add_interval_to_ts(*t, months, micros)?,
            )),
            _ => Err(ValueError::TypeMismatch.to_string()),
        }
    }
}

impl Expression for IntervalArithExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let l = self.left.evaluate(row)?;
        self.eval(&l).map_err(|e| e.into())
    }

    fn evaluate_ref<'a>(
        &'a self,
        row: &'a [ValueRef<'_>],
    ) -> Result<ValueRef<'a>, Box<dyn std::error::Error + Send + Sync>> {
        // Same shape as CASE/COALESCE/CAST: materialize and reuse the owned
        // path; Date/Timestamp/Null results are Copy and come back as
        // borrowed views directly.
        let owned: Vec<Value> = row.iter().map(ValueRef::to_value).collect();
        match self.evaluate(&owned)? {
            Value::Date(d) => Ok(ValueRef::Date(d)),
            Value::Timestamp(t) => Ok(ValueRef::Timestamp(t)),
            Value::Null => Ok(ValueRef::Null),
            _ => Err("INTERVAL arithmetic results are date-family; other shapes are a bug".into()),
        }
    }

    fn set_parameter_value(&self, param_name: &str, value: &Value) -> bool {
        self.left.set_parameter_value(param_name, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_expression() {
        let row = vec![Value::Int(42), Value::String("test".to_string())];
        let expr = ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        };
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(42));
    }

    #[test]
    fn test_constant_expression() {
        let row = vec![];
        let expr = ConstantExpression {
            value: Value::Int(999),
        };
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(999));
    }

    #[test]
    fn test_comparison_eq() {
        let row = vec![Value::Int(42)];
        let pred = ComparisonPredicate {
            left: Arc::new(ColumnExpression {
                column_name: "id".to_string(),
                column_index: 0,
            }),
            op: ComparisonOp::Eq,
            right: Arc::new(ConstantExpression {
                value: Value::Int(42),
            }),
        };
        assert!(pred.evaluate(&row).unwrap());
    }

    #[test]
    fn test_logical_and() {
        let row = vec![Value::Int(50)];
        let pred1 = Arc::new(ComparisonPredicate {
            left: Arc::new(ColumnExpression {
                column_name: "id".to_string(),
                column_index: 0,
            }),
            op: ComparisonOp::Gt,
            right: Arc::new(ConstantExpression {
                value: Value::Int(10),
            }),
        });
        let pred2 = Arc::new(ComparisonPredicate {
            left: Arc::new(ColumnExpression {
                column_name: "id".to_string(),
                column_index: 0,
            }),
            op: ComparisonOp::Lt,
            right: Arc::new(ConstantExpression {
                value: Value::Int(100),
            }),
        });
        let logical = LogicalPredicate {
            left: pred1,
            op: LogicalOp::And,
            right: pred2,
        };
        assert!(logical.evaluate(&row).unwrap());
    }

    #[test]
    fn test_parameter_expression() {
        let row = vec![];
        let expr = ParameterExpression::new("emp.dept".to_string());
        expr.set_value(Value::Int(10));
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(10));
        expr.set_value(Value::Int(20));
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(20));
        // name mismatch → no change
        assert!(!expr.set_parameter_value("wrong.name", &Value::Int(99)));
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(20));
    }
}
