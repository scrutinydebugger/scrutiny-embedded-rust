use crate::datalogging::types::AnyValAndTypePair;
use crate::types::{AnyType, VariableType, BIGGEST_SINT, BIGGEST_UINT};

/// Persistent state carried across trigger evaluations for stateful conditions.
#[derive(Clone, Copy)]
pub struct ConditionSharedData {
    /// Previous value for the ChangeMoreThan condition.
    pub cmt_previous_val: AnyType,
    pub cmt_previous_valtype: VariableType,
    pub cmt_initialized: bool,
}

impl ConditionSharedData {
    pub const DEFAULT: Self = Self {
        cmt_previous_val: AnyType { uint32: 0 },
        cmt_previous_valtype: VariableType::Uint32,
        cmt_initialized: false,
    };
}

impl Default for ConditionSharedData {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub type EvalFn = fn(&mut ConditionSharedData, &[AnyValAndTypePair]) -> bool;
pub type ResetFn = fn(&mut ConditionSharedData);

/// A fully-resolved trigger condition ready for evaluation.
#[derive(Clone, Copy)]
pub struct ActiveCondition {
    pub eval_fn: EvalFn,
    pub reset_fn: Option<ResetFn>,
    pub operand_count: u8,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Categories used after `convert_to_compare_type` reduces the valtype.
#[derive(PartialEq, Eq)]
enum CompareCategory {
    Float,
    Uint,
    Sint,
    Unknown,
}

fn compare_category(vt: VariableType) -> CompareCategory {
    if vt == VariableType::Float32 {
        CompareCategory::Float
    } else if vt == BIGGEST_UINT {
        CompareCategory::Uint
    } else if vt == BIGGEST_SINT {
        CompareCategory::Sint
    } else {
        CompareCategory::Unknown
    }
}

/// Generic relational comparison applied to a pair of `AnyValAndTypePair` values.
/// Expects both pairs to have been run through `convert_to_compare_type`.
fn relational_compare(
    op0: &AnyValAndTypePair,
    op1: &AnyValAndTypePair,
    cmp: fn(f32, f32) -> bool,
    cmp_uint: fn(crate::types::UintBiggest, crate::types::UintBiggest) -> bool,
    cmp_sint: fn(crate::types::IntBiggest, crate::types::IntBiggest) -> bool,
) -> bool {
    use CompareCategory::*;
    // Safety: after convert_to_compare_type the right union field is valid for the given valtype.
    match (compare_category(op0.valtype), compare_category(op1.valtype)) {
        (Float, Float) => cmp(unsafe { op0.val.float32 }, unsafe { op1.val.float32 }),
        (Float, Sint) => cmp(unsafe { op0.val.float32 }, unsafe { read_sint_as_float(op1) }),
        (Float, Uint) => cmp(unsafe { op0.val.float32 }, unsafe { read_uint_as_float(op1) }),
        (Sint, Float) => cmp(unsafe { read_sint_as_float(op0) }, unsafe { op1.val.float32 }),
        (Sint, Sint) => cmp_sint(unsafe { read_sint(op0) }, unsafe { read_sint(op1) }),
        (Sint, Uint) => {
            // Clamp uint to INT_MAX to avoid sign issues
            let uint_clamped = clamp_uint_to_int_max(unsafe { read_uint(op1) });
            cmp_sint(unsafe { read_sint(op0) }, uint_clamped)
        }
        (Uint, Float) => cmp(unsafe { read_uint_as_float(op0) }, unsafe { op1.val.float32 }),
        (Uint, Sint) => {
            let uint_clamped = clamp_uint_to_int_max(unsafe { read_uint(op0) });
            cmp_sint(uint_clamped, unsafe { read_sint(op1) })
        }
        (Uint, Uint) => cmp_uint(unsafe { read_uint(op0) }, unsafe { read_uint(op1) }),
        _ => false,
    }
}

#[inline]
fn clamp_uint_to_int_max(v: crate::types::UintBiggest) -> crate::types::IntBiggest {
    let max = crate::types::IntBiggest::MAX as crate::types::UintBiggest;
    if v > max { crate::types::IntBiggest::MAX } else { v as crate::types::IntBiggest }
}

/// # Safety
/// Caller must have called `convert_to_compare_type` and confirmed `valtype == BIGGEST_SINT`.
#[inline]
unsafe fn read_sint(p: &AnyValAndTypePair) -> crate::types::IntBiggest {
    crate::tools::read_biggest_sint(&p.val)
}

/// # Safety
/// Caller must have called `convert_to_compare_type` and confirmed `valtype == BIGGEST_UINT`.
#[inline]
unsafe fn read_uint(p: &AnyValAndTypePair) -> crate::types::UintBiggest {
    crate::tools::read_biggest_uint(&p.val)
}

/// # Safety
/// Caller must have called `convert_to_compare_type` and confirmed `valtype == BIGGEST_SINT`.
#[inline]
unsafe fn read_sint_as_float(p: &AnyValAndTypePair) -> f32 {
    crate::tools::read_biggest_sint(&p.val) as f32
}

/// # Safety
/// Caller must have called `convert_to_compare_type` and confirmed `valtype == BIGGEST_UINT`.
#[inline]
unsafe fn read_uint_as_float(p: &AnyValAndTypePair) -> f32 {
    crate::tools::read_biggest_uint(&p.val) as f32
}

// ── Condition evaluators ──────────────────────────────────────────────────────

pub fn always_true_eval(_data: &mut ConditionSharedData, _ops: &[AnyValAndTypePair]) -> bool {
    true
}

pub fn equal_eval(_data: &mut ConditionSharedData, ops: &[AnyValAndTypePair]) -> bool {
    if ops.len() < 2 {
        return false;
    }
    relational_compare(&ops[0], &ops[1], |a, b| a == b, |a, b| a == b, |a, b| a == b)
}

pub fn not_equal_eval(_data: &mut ConditionSharedData, ops: &[AnyValAndTypePair]) -> bool {
    if ops.len() < 2 {
        return false;
    }
    relational_compare(&ops[0], &ops[1], |a, b| a != b, |a, b| a != b, |a, b| a != b)
}

pub fn greater_than_eval(_data: &mut ConditionSharedData, ops: &[AnyValAndTypePair]) -> bool {
    if ops.len() < 2 {
        return false;
    }
    relational_compare(&ops[0], &ops[1], |a, b| a > b, |a, b| a > b, |a, b| a > b)
}

pub fn greater_or_equal_than_eval(
    _data: &mut ConditionSharedData,
    ops: &[AnyValAndTypePair],
) -> bool {
    if ops.len() < 2 {
        return false;
    }
    relational_compare(&ops[0], &ops[1], |a, b| a >= b, |a, b| a >= b, |a, b| a >= b)
}

pub fn less_than_eval(_data: &mut ConditionSharedData, ops: &[AnyValAndTypePair]) -> bool {
    if ops.len() < 2 {
        return false;
    }
    relational_compare(&ops[0], &ops[1], |a, b| a < b, |a, b| a < b, |a, b| a < b)
}

pub fn less_or_equal_than_eval(
    _data: &mut ConditionSharedData,
    ops: &[AnyValAndTypePair],
) -> bool {
    if ops.len() < 2 {
        return false;
    }
    relational_compare(&ops[0], &ops[1], |a, b| a <= b, |a, b| a <= b, |a, b| a <= b)
}

pub fn change_more_than_reset(data: &mut ConditionSharedData) {
    data.cmt_initialized = false;
}

pub fn change_more_than_eval(data: &mut ConditionSharedData, ops: &[AnyValAndTypePair]) -> bool {
    if ops.len() < 2 {
        return false;
    }

    // Operand 1 is the threshold delta (always treated as float for simplicity).
    let delta: f32 = match compare_category(ops[1].valtype) {
        CompareCategory::Float => unsafe { ops[1].val.float32 },
        CompareCategory::Uint => unsafe { read_uint_as_float(&ops[1]) },
        CompareCategory::Sint => unsafe { read_sint_as_float(&ops[1]) },
        CompareCategory::Unknown => return false,
    };

    let mut outval = false;

    if data.cmt_initialized {
        outval = match compare_category(ops[0].valtype) {
            CompareCategory::Uint => {
                let cur = unsafe { read_uint(&ops[0]) };
                let prev = crate::tools::read_biggest_uint(&data.cmt_previous_val);
                let threshold = prev.wrapping_add(delta as crate::types::IntBiggest as crate::types::UintBiggest);
                if delta >= 0.0 { cur > threshold } else { cur < threshold }
            }
            CompareCategory::Sint => {
                let cur = unsafe { read_sint(&ops[0]) };
                let prev = crate::tools::read_biggest_sint(&data.cmt_previous_val);
                let threshold = prev.wrapping_add(delta as crate::types::IntBiggest);
                if delta >= 0.0 { cur > threshold } else { cur < threshold }
            }
            CompareCategory::Float => {
                let cur = unsafe { ops[0].val.float32 };
                let prev = unsafe { data.cmt_previous_val.float32 };
                if delta >= 0.0 { cur > prev + delta } else { cur < prev + delta }
            }
            CompareCategory::Unknown => false,
        };
    } else {
        data.cmt_initialized = true;
    }

    // Store current value as previous for next evaluation.
    data.cmt_previous_val = ops[0].val;
    data.cmt_previous_valtype = ops[0].valtype;

    outval
}

pub fn is_within_eval(_data: &mut ConditionSharedData, ops: &[AnyValAndTypePair]) -> bool {
    if ops.len() < 3 {
        return false;
    }
    let as_float = |p: &AnyValAndTypePair| match compare_category(p.valtype) {
        CompareCategory::Float => unsafe { p.val.float32 },
        CompareCategory::Uint => unsafe { read_uint_as_float(p) },
        CompareCategory::Sint => unsafe { read_sint_as_float(p) },
        CompareCategory::Unknown => f32::NAN,
    };
    let v0 = as_float(&ops[0]);
    let v1 = as_float(&ops[1]);
    let margin = as_float(&ops[2]).abs();
    (v0 - v1).abs() <= margin
}

// ── Condition LUT ─────────────────────────────────────────────────────────────

use crate::datalogging::types::SupportedTriggerConditions;

/// Returns the `ActiveCondition` matching `cond`, or `None` for an unknown value.
#[must_use]
pub fn active_condition_for(cond: SupportedTriggerConditions) -> ActiveCondition {
    // Keep in sync with `SupportedTriggerConditions` enum order.
    static LUT: [ActiveCondition; 9] = [
        ActiveCondition { eval_fn: always_true_eval, reset_fn: None, operand_count: 0 },
        ActiveCondition { eval_fn: equal_eval, reset_fn: None, operand_count: 2 },
        ActiveCondition { eval_fn: not_equal_eval, reset_fn: None, operand_count: 2 },
        ActiveCondition { eval_fn: less_than_eval, reset_fn: None, operand_count: 2 },
        ActiveCondition { eval_fn: less_or_equal_than_eval, reset_fn: None, operand_count: 2 },
        ActiveCondition { eval_fn: greater_than_eval, reset_fn: None, operand_count: 2 },
        ActiveCondition { eval_fn: greater_or_equal_than_eval, reset_fn: None, operand_count: 2 },
        ActiveCondition {
            eval_fn: change_more_than_eval,
            reset_fn: Some(change_more_than_reset),
            operand_count: 2,
        },
        ActiveCondition { eval_fn: is_within_eval, reset_fn: None, operand_count: 3 },
    ];
    LUT[cond as usize]
}
