use crate::datalogging::types::AnyValAndTypePair;
use crate::types::{AnyType, VariableType, BIGGEST_SINT, BIGGEST_UINT};

/// Persistent state carried across trigger evaluations for stateful conditions.
#[derive(Clone, Copy)]
pub struct ConditionSharedData {
    /// Previous value for the `ChangeMoreThan` condition.
    pub cmt_previous_val: AnyType,
    pub cmt_initialized: bool,
}

impl ConditionSharedData {
    pub const DEFAULT: Self = Self {
        cmt_previous_val: AnyType { uint32: 0 },
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
    cmp_signed: fn(crate::types::IntBiggest, crate::types::IntBiggest) -> bool,
) -> bool {
    use CompareCategory::{Float, Sint, Uint};
    // Safety: after convert_to_compare_type the right union field is valid for the given valtype.
    match (compare_category(op0.valtype), compare_category(op1.valtype)) {
        (Float, Float) => cmp(unsafe { op0.val.float32 }, unsafe { op1.val.float32 }),
        (Float, Sint) => cmp(unsafe { op0.val.float32 }, unsafe { read_sint_as_float(op1) }),
        (Float, Uint) => cmp(unsafe { op0.val.float32 }, unsafe { read_uint_as_float(op1) }),
        (Sint, Float) => cmp(unsafe { read_sint_as_float(op0) }, unsafe { op1.val.float32 }),
        (Sint, Sint) => cmp_signed(unsafe { read_sint(op0) }, unsafe { read_sint(op1) }),
        (Sint, Uint) => {
            // Clamp uint to INT_MAX to avoid sign issues
            let uint_clamped = clamp_uint_to_int_max(unsafe { read_uint(op1) });
            cmp_signed(unsafe { read_sint(op0) }, uint_clamped)
        }
        (Uint, Float) => cmp(unsafe { read_uint_as_float(op0) }, unsafe { op1.val.float32 }),
        (Uint, Sint) => {
            let uint_clamped = clamp_uint_to_int_max(unsafe { read_uint(op0) });
            cmp_signed(uint_clamped, unsafe { read_sint(op1) })
        }
        (Uint, Uint) => cmp_uint(unsafe { read_uint(op0) }, unsafe { read_uint(op1) }),
        _ => false,
    }
}

#[inline]
const fn clamp_uint_to_int_max(v: crate::types::UintBiggest) -> crate::types::IntBiggest {
    let max = crate::types::IntBiggest::MAX as crate::types::UintBiggest;
    if v > max { crate::types::IntBiggest::MAX } else { v.cast_signed() }
}

/// # Safety
/// Caller must have called `convert_to_compare_type` and confirmed `valtype == BIGGEST_SINT`.
#[inline]
const unsafe fn read_sint(p: &AnyValAndTypePair) -> crate::types::IntBiggest {
    crate::tools::read_biggest_sint(&p.val)
}

/// # Safety
/// Caller must have called `convert_to_compare_type` and confirmed `valtype == BIGGEST_UINT`.
#[inline]
const unsafe fn read_uint(p: &AnyValAndTypePair) -> crate::types::UintBiggest {
    crate::tools::read_biggest_uint(&p.val)
}

/// # Safety
/// Caller must have called `convert_to_compare_type` and confirmed `valtype == BIGGEST_SINT`.
#[allow(clippy::cast_precision_loss)]
#[inline]
const unsafe fn read_sint_as_float(p: &AnyValAndTypePair) -> f32 {
    crate::tools::read_biggest_sint(&p.val) as f32
}

/// # Safety
/// Caller must have called `convert_to_compare_type` and confirmed `valtype == BIGGEST_UINT`.
#[allow(clippy::cast_precision_loss)]
#[inline]
const unsafe fn read_uint_as_float(p: &AnyValAndTypePair) -> f32 {
    crate::tools::read_biggest_uint(&p.val) as f32
}

// ── Condition evaluators ──────────────────────────────────────────────────────

pub const fn always_true_eval(_data: &mut ConditionSharedData, _ops: &[AnyValAndTypePair]) -> bool {
    true
}

#[allow(clippy::float_cmp)]
pub fn equal_eval(_data: &mut ConditionSharedData, ops: &[AnyValAndTypePair]) -> bool {
    if ops.len() < 2 {
        return false;
    }
    relational_compare(&ops[0], &ops[1], |a, b| a == b, |a, b| a == b, |a, b| a == b)
}

#[allow(clippy::float_cmp)]
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

pub const fn change_more_than_reset(data: &mut ConditionSharedData) {
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

    let outval = if data.cmt_initialized {
        match compare_category(ops[0].valtype) {
            CompareCategory::Uint => {
                let cur = unsafe { read_uint(&ops[0]) };
                let prev = crate::tools::read_biggest_uint(&data.cmt_previous_val);
                // Cast: f32 → signed delta → unsigned so wrapping_add subtracts when delta < 0.
                #[allow(clippy::cast_possible_truncation)]
                let threshold = prev.wrapping_add((delta as crate::types::IntBiggest).cast_unsigned());
                if delta >= 0.0 { cur > threshold } else { cur < threshold }
            }
            CompareCategory::Sint => {
                let cur = unsafe { read_sint(&ops[0]) };
                let prev = crate::tools::read_biggest_sint(&data.cmt_previous_val);
                #[allow(clippy::cast_possible_truncation)]
                let threshold = prev.wrapping_add(delta as crate::types::IntBiggest);
                if delta >= 0.0 { cur > threshold } else { cur < threshold }
            }
            CompareCategory::Float => {
                let cur = unsafe { ops[0].val.float32 };
                let prev = unsafe { data.cmt_previous_val.float32 };
                if delta >= 0.0 { cur > prev + delta } else { cur < prev + delta }
            }
            CompareCategory::Unknown => false,
        }
    } else {
        data.cmt_initialized = true;
        false
    };

    // Store current value as previous for next evaluation.
    data.cmt_previous_val = ops[0].val;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AnyType, IntBiggest, UintBiggest, BIGGEST_SINT, BIGGEST_UINT};

    fn fp(f: f32) -> AnyValAndTypePair {
        AnyValAndTypePair { val: AnyType { float32: f }, valtype: crate::types::VariableType::Float32 }
    }

    fn up(u: UintBiggest) -> AnyValAndTypePair {
        let mut val = AnyType { uint32: 0 };
        crate::tools::set_biggest_uint(&mut val, u);
        AnyValAndTypePair { val, valtype: BIGGEST_UINT }
    }

    fn sp(s: IntBiggest) -> AnyValAndTypePair {
        let mut val = AnyType { uint32: 0 };
        crate::tools::set_biggest_sint(&mut val, s);
        AnyValAndTypePair { val, valtype: BIGGEST_SINT }
    }

    fn csd() -> ConditionSharedData { ConditionSharedData::DEFAULT }

    #[test]
    fn always_true() {
        assert!(always_true_eval(&mut csd(), &[]));
    }

    #[test]
    fn operator_eq() {
        let mut cd = csd();
        assert!(!equal_eval(&mut cd, &[fp(0.4), fp(0.5)]));
        assert!( equal_eval(&mut cd, &[fp(0.5), fp(0.5)]));
    }

    #[test]
    fn operator_neq() {
        let mut cd = csd();
        assert!(!not_equal_eval(&mut cd, &[fp(0.5), fp(0.5)]));
        assert!( not_equal_eval(&mut cd, &[fp(0.4), fp(0.5)]));
    }

    #[test]
    fn operator_gt() {
        let mut cd = csd();
        assert!(!greater_than_eval(&mut cd, &[fp(0.4), fp(0.5)]));
        assert!(!greater_than_eval(&mut cd, &[fp(0.5), fp(0.5)]));
        assert!( greater_than_eval(&mut cd, &[fp(0.6), fp(0.5)]));
    }

    #[test]
    fn operator_get() {
        let mut cd = csd();
        assert!(!greater_or_equal_than_eval(&mut cd, &[fp(0.4), fp(0.5)]));
        assert!( greater_or_equal_than_eval(&mut cd, &[fp(0.5), fp(0.5)]));
        assert!( greater_or_equal_than_eval(&mut cd, &[fp(0.6), fp(0.5)]));
    }

    #[test]
    fn operator_lt() {
        let mut cd = csd();
        assert!(!less_than_eval(&mut cd, &[fp(0.6), fp(0.5)]));
        assert!(!less_than_eval(&mut cd, &[fp(0.5), fp(0.5)]));
        assert!( less_than_eval(&mut cd, &[fp(0.4), fp(0.5)]));
    }

    #[test]
    fn operator_let() {
        let mut cd = csd();
        assert!(!less_or_equal_than_eval(&mut cd, &[fp(0.6), fp(0.5)]));
        assert!( less_or_equal_than_eval(&mut cd, &[fp(0.5), fp(0.5)]));
        assert!( less_or_equal_than_eval(&mut cd, &[fp(0.4), fp(0.5)]));
    }

    #[test]
    fn equality_all_types() {
        let mut cd = csd();
        for swap in [false, true] {
            let p = |a: AnyValAndTypePair, b: AnyValAndTypePair| -> [AnyValAndTypePair; 2] {
                if swap { [b, a] } else { [a, b] }
            };
            assert!(!equal_eval(&mut cd, &p(fp(0.5), fp(0.6))));
            assert!( equal_eval(&mut cd, &p(fp(0.5), fp(0.5))));
            assert!(!equal_eval(&mut cd, &p(up(2), fp(2.1))));
            assert!( equal_eval(&mut cd, &p(up(2), fp(2.0))));
            assert!(!equal_eval(&mut cd, &p(sp(2), fp(2.1))));
            assert!( equal_eval(&mut cd, &p(sp(2), fp(2.0))));
            assert!(!equal_eval(&mut cd, &p(sp(2), up(3))));
            assert!( equal_eval(&mut cd, &p(sp(2), up(2))));
            assert!(!equal_eval(&mut cd, &p(up(2), up(3))));
            assert!( equal_eval(&mut cd, &p(up(2), up(2))));
            assert!(!equal_eval(&mut cd, &p(sp(2), sp(3))));
            assert!( equal_eval(&mut cd, &p(sp(2), sp(2))));
            // sint(-1) must not equal uint(MAX): different signs
            assert!(!equal_eval(&mut cd, &p(sp(-1), up(UintBiggest::MAX))));
        }
    }

    #[test]
    fn lt_mixed_sign_and_types() {
        let mut cd = csd();
        for swap in [false, true] {
            let p = |a: AnyValAndTypePair, b: AnyValAndTypePair| -> [AnyValAndTypePair; 2] {
                if swap { [b, a] } else { [a, b] }
            };
            let exp = |n: bool| if swap { !n } else { n };

            assert_eq!(less_than_eval(&mut cd, &p(fp(1.0),  sp(-5))),              exp(false));
            assert_eq!(less_than_eval(&mut cd, &p(fp(-6.0), sp(-5))),              exp(true));
            assert_eq!(less_than_eval(&mut cd, &p(fp(-1.0), up(0))),               exp(true));
            assert_eq!(less_than_eval(&mut cd, &p(sp(-6),   sp(-5))),              exp(true));
            assert_eq!(less_than_eval(&mut cd, &p(up(UintBiggest::MAX - 1), sp(-1))), exp(false));
        }
    }

    #[test]
    fn change_more_than_basic() {
        let mut cd = csd();
        // Positive threshold
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), fp(5.0)])); // init
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), fp(5.0)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), fp(5.0)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(11.0), fp(5.0)])); // +1 < 5
        assert!(!change_more_than_eval(&mut cd, &[fp(5.0),  fp(5.0)])); // decrease: ignored
        assert!( change_more_than_eval(&mut cd, &[fp(10.1), fp(5.0)])); // +5.1 > 5
        assert!(!change_more_than_eval(&mut cd, &[fp(11.0), fp(5.0)])); // +0.9 < 5
        assert!( change_more_than_eval(&mut cd, &[fp(20.0), fp(5.0)])); // +9 > 5
        assert!( change_more_than_eval(&mut cd, &[fp(30.0), fp(5.0)])); // +10 > 5

        // Negative threshold
        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), fp(-5.0)])); // init
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), fp(-5.0)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(20.0), fp(-5.0)])); // increase: ignored
        assert!( change_more_than_eval(&mut cd, &[fp(14.9), fp(-5.0)])); // -5.1 < -5
        assert!(!change_more_than_eval(&mut cd, &[fp(14.0), fp(-5.0)])); // -0.9 not < -5
        assert!( change_more_than_eval(&mut cd, &[fp(5.0),  fp(-5.0)])); // -9 < -5
        assert!( change_more_than_eval(&mut cd, &[fp(-5.0), fp(-5.0)])); // -10 < -5
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn change_more_than_all_types() {
        let mut cd = csd();

        // ── Float + Sint ────────────────────────────────────────────────────────
        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(11.0), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(5.0),  sp(5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(10.1), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(11.0), sp(5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(20.0), sp(5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(30.0), sp(5)]));

        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), sp(-5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), sp(-5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(20.0), sp(-5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(14.9), sp(-5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(14.0), sp(-5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(5.0),  sp(-5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(-5.0), sp(-5)]));

        // ── Float + Uint ────────────────────────────────────────────────────────
        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), up(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), up(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(10.0), up(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(11.0), up(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(5.0),  up(5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(10.1), up(5)]));
        assert!(!change_more_than_eval(&mut cd, &[fp(11.0), up(5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(20.0), up(5)]));
        assert!( change_more_than_eval(&mut cd, &[fp(30.0), up(5)]));

        // ── Uint + Uint ─────────────────────────────────────────────────────────
        let mut v: UintBiggest = UintBiggest::MAX.wrapping_sub(999); // == (uint_biggest_t)-1000
        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[up(v), up(5)]));
        assert!(!change_more_than_eval(&mut cd, &[up(v), up(5)]));
        assert!(!change_more_than_eval(&mut cd, &[up(v), up(5)]));
        v = v.wrapping_add(1);
        assert!(!change_more_than_eval(&mut cd, &[up(v), up(5)]));
        v = v.wrapping_sub(10);
        assert!(!change_more_than_eval(&mut cd, &[up(v), up(5)]));
        v = v.wrapping_add(6);
        assert!( change_more_than_eval(&mut cd, &[up(v), up(5)]));
        v = v.wrapping_add(2);
        assert!(!change_more_than_eval(&mut cd, &[up(v), up(5)]));
        v = v.wrapping_add(6);
        assert!( change_more_than_eval(&mut cd, &[up(v), up(5)]));
        v = v.wrapping_add(10);
        assert!( change_more_than_eval(&mut cd, &[up(v), up(5)]));

        // ── Uint + Sint (positive threshold) ───────────────────────────────────
        v = UintBiggest::MAX.wrapping_sub(999);
        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(5)]));
        v = v.wrapping_add(1);
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(5)]));
        v = v.wrapping_sub(10);
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(5)]));
        v = v.wrapping_add(6);
        assert!( change_more_than_eval(&mut cd, &[up(v), sp(5)]));
        v = v.wrapping_add(4);
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(5)]));
        v = v.wrapping_add(10);
        assert!( change_more_than_eval(&mut cd, &[up(v), sp(5)]));
        v = v.wrapping_add(6);
        assert!( change_more_than_eval(&mut cd, &[up(v), sp(5)]));

        // ── Uint + Sint (negative threshold) ───────────────────────────────────
        v = UintBiggest::MAX.wrapping_sub(999);
        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(-5)]));
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(-5)]));
        v = v.wrapping_add(20);
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(-5)]));
        v = v.wrapping_sub(6);
        assert!( change_more_than_eval(&mut cd, &[up(v), sp(-5)]));
        v = v.wrapping_sub(4);
        assert!(!change_more_than_eval(&mut cd, &[up(v), sp(-5)]));
        v = v.wrapping_sub(100);
        assert!( change_more_than_eval(&mut cd, &[up(v), sp(-5)]));
        v = v.wrapping_sub(10);
        assert!( change_more_than_eval(&mut cd, &[up(v), sp(-5)]));

        // ── Sint + Sint (positive threshold) ───────────────────────────────────
        let mut sv: IntBiggest = -100;
        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));
        sv += 1;
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));
        sv -= 10;
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));
        sv += 6;
        assert!( change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));
        sv += 4;
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));
        sv += 10;
        assert!( change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));
        sv += 6;
        assert!( change_more_than_eval(&mut cd, &[sp(sv), sp(5)]));

        // ── Sint + Sint (negative threshold) ───────────────────────────────────
        sv = -100;
        change_more_than_reset(&mut cd);
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(-5)]));
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(-5)]));
        sv += 20;
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(-5)]));
        sv -= 6;
        assert!( change_more_than_eval(&mut cd, &[sp(sv), sp(-5)]));
        sv -= 4;
        assert!(!change_more_than_eval(&mut cd, &[sp(sv), sp(-5)]));
        sv -= 100;
        assert!( change_more_than_eval(&mut cd, &[sp(sv), sp(-5)]));
        sv -= 10;
        assert!( change_more_than_eval(&mut cd, &[sp(sv), sp(-5)]));
    }

    #[test]
    fn is_within() {
        let mut cd = csd();
        assert!(!is_within_eval(&mut cd, &[fp(10.0), fp(5.0),   fp(1.0)])); // |10-5|=5 > 1
        assert!(!is_within_eval(&mut cd, &[fp(10.0), fp(11.01), fp(1.0)])); // 1.01 > 1
        assert!( is_within_eval(&mut cd, &[fp(10.0), fp(11.0),  fp(1.0)])); // 1.0 <= 1
        assert!(!is_within_eval(&mut cd, &[fp(10.0), fp(8.99),  fp(1.0)])); // 1.01 > 1
        assert!( is_within_eval(&mut cd, &[fp(10.0), fp(9.0),   fp(1.0)])); // 1.0 <= 1
    }
}


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
