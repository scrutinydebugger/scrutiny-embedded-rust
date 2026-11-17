#![cfg(feature = "datalogging")]

use scrutiny_embedded_rust::datalogging::trigger::{
    always_true_eval, change_more_than_eval, change_more_than_reset, equal_eval,
    greater_or_equal_than_eval, greater_than_eval, is_within_eval, less_or_equal_than_eval,
    less_than_eval, not_equal_eval, ConditionSharedData,
};
use scrutiny_embedded_rust::datalogging::AnyValAndTypePair;
use scrutiny_embedded_rust::tools::{set_biggest_sint, set_biggest_uint};
use scrutiny_embedded_rust::types::{AnyType, IntBiggest, UintBiggest, VariableType, BIGGEST_SINT, BIGGEST_UINT};

fn fp(v: f32) -> AnyValAndTypePair {
    AnyValAndTypePair { val: AnyType { float32: v }, valtype: VariableType::Float32 }
}
fn sp(v: IntBiggest) -> AnyValAndTypePair {
    let mut p = AnyValAndTypePair::DEFAULT;
    set_biggest_sint(&mut p.val, v);
    p.valtype = BIGGEST_SINT;
    p
}
fn up(v: UintBiggest) -> AnyValAndTypePair {
    let mut p = AnyValAndTypePair::DEFAULT;
    set_biggest_uint(&mut p.val, v);
    p.valtype = BIGGEST_UINT;
    p
}
fn ord2(swap: bool, a: AnyValAndTypePair, b: AnyValAndTypePair) -> [AnyValAndTypePair; 2] {
    if swap { [b, a] } else { [a, b] }
}

#[test]
fn always_true() {
    let mut d = ConditionSharedData::default();
    assert!(always_true_eval(&mut d, &[]));
    assert!(always_true_eval(&mut d, &[fp(0.0)]));
}

#[test]
fn operator_eq() {
    let mut d = ConditionSharedData::default();
    assert!(!equal_eval(&mut d, &[fp(0.4), fp(0.5)]));
    assert!(equal_eval(&mut d, &[fp(0.5), fp(0.5)]));
}

#[test]
fn operator_neq() {
    let mut d = ConditionSharedData::default();
    assert!(!not_equal_eval(&mut d, &[fp(0.5), fp(0.5)]));
    assert!(not_equal_eval(&mut d, &[fp(0.4), fp(0.5)]));
}

#[test]
fn operator_gt() {
    let mut d = ConditionSharedData::default();
    assert!(!greater_than_eval(&mut d, &[fp(0.4), fp(0.5)]));
    assert!(!greater_than_eval(&mut d, &[fp(0.5), fp(0.5)]));
    assert!(greater_than_eval(&mut d, &[fp(0.6), fp(0.5)]));
}

#[test]
fn operator_get() {
    let mut d = ConditionSharedData::default();
    assert!(!greater_or_equal_than_eval(&mut d, &[fp(0.4), fp(0.5)]));
    assert!(greater_or_equal_than_eval(&mut d, &[fp(0.5), fp(0.5)]));
    assert!(greater_or_equal_than_eval(&mut d, &[fp(0.6), fp(0.5)]));
}

#[test]
fn operator_lt() {
    let mut d = ConditionSharedData::default();
    assert!(!less_than_eval(&mut d, &[fp(0.6), fp(0.5)]));
    assert!(!less_than_eval(&mut d, &[fp(0.5), fp(0.5)]));
    assert!(less_than_eval(&mut d, &[fp(0.4), fp(0.5)]));
}

#[test]
fn operator_let() {
    let mut d = ConditionSharedData::default();
    assert!(!less_or_equal_than_eval(&mut d, &[fp(0.6), fp(0.5)]));
    assert!(less_or_equal_than_eval(&mut d, &[fp(0.5), fp(0.5)]));
    assert!(less_or_equal_than_eval(&mut d, &[fp(0.4), fp(0.5)]));
}

#[test]
fn equality_all_types() {
    let mut d = ConditionSharedData::default();
    for swap in [false, true] {
        // float / float
        assert!(!equal_eval(&mut d, &ord2(swap, fp(0.5), fp(0.6))));
        assert!(equal_eval(&mut d, &ord2(swap, fp(0.5), fp(0.5))));

        // uint / float
        assert!(!equal_eval(&mut d, &ord2(swap, up(2), fp(2.1))));
        assert!(equal_eval(&mut d, &ord2(swap, up(2), fp(2.0))));

        // sint / float
        assert!(!equal_eval(&mut d, &ord2(swap, sp(2), fp(2.1))));
        assert!(equal_eval(&mut d, &ord2(swap, sp(2), fp(2.0))));

        // sint / uint
        assert!(!equal_eval(&mut d, &ord2(swap, sp(2), up(3))));
        assert!(equal_eval(&mut d, &ord2(swap, sp(2), up(2))));

        // uint / uint
        assert!(!equal_eval(&mut d, &ord2(swap, up(2), up(3))));
        assert!(equal_eval(&mut d, &ord2(swap, up(2), up(2))));

        // sint / sint
        assert!(!equal_eval(&mut d, &ord2(swap, sp(2), sp(3))));
        assert!(equal_eval(&mut d, &ord2(swap, sp(2), sp(2))));

        // sint(-1) vs uint(UINT_MAX): clamped to INT_MAX, not equal to -1
        assert!(!equal_eval(&mut d, &ord2(swap, sp(-1), up(UintBiggest::MAX))));
    }
}

#[test]
fn lt_mixed_sign_and_types() {
    let mut d = ConditionSharedData::default();
    for swap in [false, true] {
        let truefalse = swap;

        // float(1.0) vs sint(-5): lt(1.0,-5)=false / lt(-5,1.0)=true
        assert_eq!(less_than_eval(&mut d, &ord2(swap, fp(1.0), sp(-5))), truefalse);
        // float(-6.0) vs sint(-5): lt(-6,-5)=true / lt(-5,-6)=false
        assert_eq!(less_than_eval(&mut d, &ord2(swap, fp(-6.0), sp(-5))), !truefalse);

        // float(-1.0) vs uint(0): lt(-1.0,0)=true / lt(0,-1.0)=false
        assert_eq!(less_than_eval(&mut d, &ord2(swap, fp(-1.0), up(0))), !truefalse);

        // sint(-6) vs sint(-5): lt(-6,-5)=true / lt(-5,-6)=false
        assert_eq!(less_than_eval(&mut d, &ord2(swap, sp(-6), sp(-5))), !truefalse);

        // uint(UINT_MAX-1) vs sint(-1): clamped→lt(INT_MAX,-1)=false / lt(-1,INT_MAX)=true
        assert_eq!(
            less_than_eval(&mut d, &ord2(swap, up(UintBiggest::MAX - 1), sp(-1))),
            truefalse
        );
    }
}

#[test]
fn change_more_than_basic() {
    let mut d = ConditionSharedData::default();
    change_more_than_reset(&mut d);

    assert!(!change_more_than_eval(&mut d, &[fp(10.0), fp(5.0)])); // init, prev=10
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), fp(5.0)])); // 10 > 15? No
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), fp(5.0)]));
    assert!(!change_more_than_eval(&mut d, &[fp(11.0), fp(5.0)])); // 11 > 15? No, prev=11
    assert!(!change_more_than_eval(&mut d, &[fp(5.0), fp(5.0)]));  // 5 > 16? No, prev=5
    assert!(change_more_than_eval(&mut d, &[fp(10.1), fp(5.0)]));  // 10.1 > 10? Yes, prev=10.1
    assert!(!change_more_than_eval(&mut d, &[fp(11.0), fp(5.0)])); // 11 > 15.1? No, prev=11
    assert!(change_more_than_eval(&mut d, &[fp(20.0), fp(5.0)]));  // 20 > 16? Yes, prev=20
    assert!(change_more_than_eval(&mut d, &[fp(30.0), fp(5.0)]));  // 30 > 25? Yes

    change_more_than_reset(&mut d);
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), fp(-5.0)])); // init, prev=10
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), fp(-5.0)])); // 10 < 5? No
    assert!(!change_more_than_eval(&mut d, &[fp(20.0), fp(-5.0)])); // 20 < 5? No, prev=20
    assert!(change_more_than_eval(&mut d, &[fp(14.9), fp(-5.0)]));  // 14.9 < 15? Yes, prev=14.9
    assert!(!change_more_than_eval(&mut d, &[fp(14.0), fp(-5.0)])); // 14 < 9.9? No, prev=14
    assert!(change_more_than_eval(&mut d, &[fp(5.0), fp(-5.0)]));   // 5 < 9? Yes, prev=5
    assert!(change_more_than_eval(&mut d, &[fp(-5.0), fp(-5.0)]));  // -5 < 0? Yes
}

#[test]
fn change_more_than_all_types() {
    let mut d = ConditionSharedData::default();

    // ── Float value, Sint threshold ──────────────────────────────────────────
    change_more_than_reset(&mut d);
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), sp(5)])); // init
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(11.0), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(5.0), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[fp(10.1), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(11.0), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[fp(20.0), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[fp(30.0), sp(5)]));

    change_more_than_reset(&mut d);
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), sp(-5)])); // init, prev=10
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), sp(-5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(20.0), sp(-5)])); // positive change, no trigger
    assert!(change_more_than_eval(&mut d, &[fp(14.9), sp(-5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(14.0), sp(-5)]));
    assert!(change_more_than_eval(&mut d, &[fp(5.0), sp(-5)]));
    assert!(change_more_than_eval(&mut d, &[fp(-5.0), sp(-5)]));

    // ── Float value, Uint threshold ──────────────────────────────────────────
    change_more_than_reset(&mut d);
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), up(5)])); // init
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(10.0), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(11.0), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(5.0), up(5)]));
    assert!(change_more_than_eval(&mut d, &[fp(10.1), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[fp(11.0), up(5)]));
    assert!(change_more_than_eval(&mut d, &[fp(20.0), up(5)]));
    assert!(change_more_than_eval(&mut d, &[fp(30.0), up(5)]));

    // ── Uint value, Uint threshold ───────────────────────────────────────────
    change_more_than_reset(&mut d);
    let b: UintBiggest = 0u32.wrapping_sub(1000) as UintBiggest;
    assert!(!change_more_than_eval(&mut d, &[up(b), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b.wrapping_add(1)), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b.wrapping_add(1).wrapping_sub(10)), up(5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_sub(9).wrapping_add(6)), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b.wrapping_sub(3).wrapping_add(2)), up(5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_sub(1).wrapping_add(6)), up(5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_add(5).wrapping_add(10)), up(5)]));

    // ── Uint value, Sint threshold (positive) ───────────────────────────────
    change_more_than_reset(&mut d);
    let b: UintBiggest = 0u32.wrapping_sub(1000) as UintBiggest;
    assert!(!change_more_than_eval(&mut d, &[up(b), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b.wrapping_add(1)), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b.wrapping_add(1).wrapping_sub(10)), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_sub(9).wrapping_add(6)), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b.wrapping_sub(3).wrapping_add(4)), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_add(1).wrapping_add(10)), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_add(11).wrapping_add(6)), sp(5)]));

    // ── Uint value, Sint threshold (negative) ───────────────────────────────
    change_more_than_reset(&mut d);
    let b: UintBiggest = 0u32.wrapping_sub(1000) as UintBiggest;
    assert!(!change_more_than_eval(&mut d, &[up(b), sp(-5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b), sp(-5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b.wrapping_add(20)), sp(-5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_add(20).wrapping_sub(6)), sp(-5)]));
    assert!(!change_more_than_eval(&mut d, &[up(b.wrapping_add(14).wrapping_sub(4)), sp(-5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_add(10).wrapping_sub(100)), sp(-5)]));
    assert!(change_more_than_eval(&mut d, &[up(b.wrapping_sub(90).wrapping_sub(10)), sp(-5)]));

    // ── Sint value, Sint threshold (positive) ───────────────────────────────
    change_more_than_reset(&mut d);
    assert!(!change_more_than_eval(&mut d, &[sp(-100), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-100), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-100), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-99), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-109), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-103), sp(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-99), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-89), sp(5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-83), sp(5)]));

    // ── Sint value, Sint threshold (negative) ───────────────────────────────
    change_more_than_reset(&mut d);
    assert!(!change_more_than_eval(&mut d, &[sp(-100), sp(-5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-100), sp(-5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-80), sp(-5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-86), sp(-5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-90), sp(-5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-190), sp(-5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-200), sp(-5)]));

    // ── Sint value, Uint threshold (positive) ───────────────────────────────
    change_more_than_reset(&mut d);
    assert!(!change_more_than_eval(&mut d, &[sp(-100), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-100), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-100), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-99), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-109), up(5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-103), up(5)]));
    assert!(!change_more_than_eval(&mut d, &[sp(-99), up(5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-89), up(5)]));
    assert!(change_more_than_eval(&mut d, &[sp(-83), up(5)]));
}

#[test]
fn is_within() {
    let mut d = ConditionSharedData::default();
    assert!(!is_within_eval(&mut d, &[fp(10.0), fp(5.0), fp(1.0)]));
    assert!(!is_within_eval(&mut d, &[fp(10.0), fp(11.01), fp(1.0)]));
    assert!(is_within_eval(&mut d, &[fp(10.0), fp(11.0), fp(1.0)]));
    assert!(!is_within_eval(&mut d, &[fp(10.0), fp(8.99), fp(1.0)]));
    assert!(is_within_eval(&mut d, &[fp(10.0), fp(9.0), fp(1.0)]));
}
