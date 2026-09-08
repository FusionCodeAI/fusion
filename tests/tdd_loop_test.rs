//! Integration and regression tests for autonomous TDD Loop engine (arXiv:2608.26263).
//!
//! Validates:
//! 1. Test failure parsing across multiple ecosystems:
//!    - Rust `cargo test` failure outputs (panics, assertions, compiler feedback).
//!    - Python `pytest` failure outputs (traceback blocks, E-prefixed assertions, short summaries).
//!    - Node.js / Bun `vitest` and `jest` outputs (FAIL headers, bullet markers, stack frames).
//! 2. TDD phase state transitions from Red to Green, Refactor, and Completed.
//! 3. Correction guidance and targeted fix constraint formulation.
//! 4. TddEngine iteration tracking and history recording.

#[path = "../src/agent/tdd_loop.rs"]
pub mod tdd_loop;

use tdd_loop::{strip_ansi, TddEngine, TddIteration, TddPhase, TestFailure};

// ============================================================================
// Test 1: Cargo Test Failure Parsing
// ============================================================================

#[test]
fn test_cargo_test_assertion_failure_parsing() {
    let raw_cargo_output = r#"
running 3 tests
test tests::test_add ... ok
test tests::test_sub ... FAILED
test tests::test_mul ... ok

failures:

---- tests::test_sub stdout ----
thread 'tests::test_sub' panicked at src/lib.rs:42:5:
assertion `left == right` failed
  left: 3
 right: 5
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

failures:
    tests::test_sub

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"#;

    let (passed, failures) = TddEngine::parse_test_output(raw_cargo_output, 101);
    assert!(!passed, "Iteration must be marked as failed");
    assert_eq!(failures.len(), 1, "Expected exactly 1 test failure");

    let failure = &failures[0];
    assert_eq!(failure.test_name, "tests::test_sub");
    assert_eq!(failure.file.as_deref(), Some("src/lib.rs"));
    assert_eq!(failure.line, Some(42));
    assert!(failure
        .error_message
        .contains("assertion `left == right` failed"));
    assert!(failure.error_message.contains("left: 3"));
    assert!(failure.error_message.contains("right: 5"));
}

#[test]
fn test_cargo_test_quoted_panic_and_backtrace() {
    let raw_cargo_output = r#"
failures:

---- tests::test_division_by_zero stdout ----
thread 'tests::test_division_by_zero' panicked at 'attempt to divide by zero', src/calc.rs:15:9
stack backtrace:
   0: rust_begin_unwind
   1: core::panicking::panic_fmt
   2: calc::div
             at src/calc.rs:15:9
note: Some details are omitted, run with `RUST_BACKTRACE=full` for a verbose backtrace.

failures:
    tests::test_division_by_zero

test result: FAILED. 0 passed; 1 failed; 0 ignored; finished in 0.00s
"#;

    let (passed, failures) = TddEngine::parse_test_output(raw_cargo_output, 101);
    assert!(!passed);
    assert_eq!(failures.len(), 1);

    let failure = &failures[0];
    assert_eq!(failure.test_name, "tests::test_division_by_zero");
    assert_eq!(failure.file.as_deref(), Some("src/calc.rs"));
    assert_eq!(failure.line, Some(15));
    assert_eq!(failure.error_message, "attempt to divide by zero");
    assert!(failure.stack_trace.is_some());
    assert!(failure
        .stack_trace
        .as_deref()
        .unwrap()
        .contains("rust_begin_unwind"));
}

#[test]
fn test_cargo_compiler_error_feedback() {
    let raw_output = r#"
error[E0425]: cannot find value `calculate_total` in this scope
  --> src/billing.rs:24:9
   |
24 |     let total = calculate_total(items);
   |                 ^^^^^^^^^^^^^^^ not found in this scope

For more information about this error, try `rustc --explain E0425`.
error: could not compile `fusion` due to 1 previous error
"#;

    let (passed, failures) = TddEngine::parse_test_output(raw_output, 1);
    assert!(!passed);
    assert_eq!(failures.len(), 1);

    let failure = &failures[0];
    assert_eq!(failure.test_name, "compiler::error");
    assert_eq!(failure.file.as_deref(), Some("src/billing.rs"));
    assert_eq!(failure.line, Some(24));
    assert!(failure.error_message.contains("error[E0425]"));
    assert!(failure
        .error_message
        .contains("cannot find value `calculate_total`"));
}

// ============================================================================
// Test 2: Pytest Failure Parsing
// ============================================================================

#[test]
fn test_pytest_assertion_failure_parsing() {
    let raw_pytest_output = r#"
============================= test session starts ==============================
platform darwin -- Python 3.12.0, pytest-8.1.1, pluggy-1.4.0
rootdir: /workspace
collected 3 items

tests/test_math.py .F.                                                   [100%]

=================================== FAILURES ===================================
___________________________________ test_add ___________________________________

    def test_add():
>       assert add(1, 2) == 4
E       assert 3 == 4
E        +  where 3 = add(1, 2)

tests/test_math.py:15: AssertionError
=========================== short test summary info ============================
FAILED tests/test_math.py::test_add - assert 3 == 4
========================= 1 failed, 2 passed in 0.12s ==========================
"#;

    let (passed, failures) = TddEngine::parse_test_output(raw_pytest_output, 1);
    assert!(!passed);
    assert_eq!(failures.len(), 1);

    let failure = &failures[0];
    assert_eq!(failure.test_name, "test_add");
    assert_eq!(failure.file.as_deref(), Some("tests/test_math.py"));
    assert_eq!(failure.line, Some(15));
    assert!(failure.error_message.contains("assert 3 == 4"));
    assert!(failure.error_message.contains("+  where 3 = add(1, 2)"));
    assert!(failure.stack_trace.is_some());
}

#[test]
fn test_pytest_exception_and_traceback() {
    let raw_pytest_output = r#"
=================================== FAILURES ===================================
______________________________ TestOps.test_divide _____________________________

self = <tests.test_ops.TestOps object at 0x104>

    def test_divide(self):
>       divide(10, 0)

tests/test_ops.py:25: 
_ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ 

    def divide(a, b):
>       raise ZeroDivisionError("division by zero")
E       ZeroDivisionError: division by zero

src/calc.py:8: ZeroDivisionError
=========================== short test summary info ============================
FAILED tests/test_ops.py::TestOps::test_divide - ZeroDivisionError: division by zero
============================== 1 failed in 0.02s ===============================
"#;

    let (passed, failures) = TddEngine::parse_test_output(raw_pytest_output, 1);
    assert!(!passed);
    assert_eq!(failures.len(), 1);

    let failure = &failures[0];
    assert_eq!(failure.test_name, "TestOps.test_divide");
    assert_eq!(failure.file.as_deref(), Some("tests/test_ops.py"));
    assert_eq!(failure.line, Some(25));
    assert!(failure
        .error_message
        .contains("ZeroDivisionError: division by zero"));
}

#[test]
fn test_pytest_short_summary_fallback() {
    let raw_pytest_output = r#"
=========================== short test summary info ============================
FAILED tests/test_account.py::test_withdraw_insufficient_funds - InsufficientFundsError: balance is 0
FAILED tests/test_account.py::test_negative_deposit - ValueError: deposit cannot be negative
============================== 2 failed in 0.03s ===============================
"#;

    let (passed, failures) = TddEngine::parse_test_output(raw_pytest_output, 1);
    assert!(!passed);
    assert_eq!(failures.len(), 2);

    assert_eq!(failures[0].test_name, "test_withdraw_insufficient_funds");
    assert_eq!(failures[0].file.as_deref(), Some("tests/test_account.py"));
    assert!(failures[0]
        .error_message
        .contains("InsufficientFundsError: balance is 0"));

    assert_eq!(failures[1].test_name, "test_negative_deposit");
    assert_eq!(failures[1].file.as_deref(), Some("tests/test_account.py"));
    assert!(failures[1]
        .error_message
        .contains("ValueError: deposit cannot be negative"));
}

// ============================================================================
// Test 3: Vitest and Jest Failure Parsing
// ============================================================================

#[test]
fn test_vitest_failure_parsing() {
    let raw_vitest_output = r#"
FAIL  src/sum.test.ts > sum of two numbers
AssertionError: expected 3 to be 4 // Object.is equality

- Expected
+ Received

- 4
+ 3 

 ❯ src/sum.test.ts:7:14
      5| test('sum of two numbers', () => {
      6|   expect(sum(1, 2)).toBe(4)
      7| })
 ❯ runTest node_modules/vitest/dist/runner.js:123:45

Tests  1 failed | 2 passed (3)
"#;

    let (passed, failures) = TddEngine::parse_test_output(raw_vitest_output, 1);
    assert!(!passed);
    assert_eq!(failures.len(), 1);

    let failure = &failures[0];
    assert_eq!(failure.test_name, "sum of two numbers");
    assert_eq!(failure.file.as_deref(), Some("src/sum.test.ts"));
    assert_eq!(failure.line, Some(7));
    assert!(failure.error_message.contains("expected 3 to be 4"));
    assert!(failure.stack_trace.is_some());
}

#[test]
fn test_jest_failure_parsing() {
    let raw_jest_output = r#"
FAIL src/calculator.test.ts
  ● Calculator › adds two numbers

    expect(received).toBe(expected) // Object.is equality

    Expected: 4
    Received: 3

      12 |   it('adds two numbers', () => {
    > 13 |     expect(add(1, 2)).toBe(4);
         |                       ^
      14 |   });

      at Object.toBe (src/calculator.test.ts:13:23)

  ● Calculator › handles divide by zero

    Error: Division by zero

      at Object.divide (src/calculator.ts:8:11)
      at Object.<anonymous> (src/calculator.test.ts:25:14)

Test Suites: 1 failed, 1 total
Tests:       2 failed, 3 passed, 5 total
"#;

    let (passed, failures) = TddEngine::parse_test_output(raw_jest_output, 1);
    assert!(!passed);
    assert_eq!(failures.len(), 2);

    let failure1 = &failures[0];
    assert_eq!(failure1.test_name, "Calculator › adds two numbers");
    assert_eq!(failure1.file.as_deref(), Some("src/calculator.test.ts"));
    assert_eq!(failure1.line, Some(13));
    assert!(failure1.error_message.contains("Expected: 4"));
    assert!(failure1.error_message.contains("Received: 3"));

    let failure2 = &failures[1];
    assert_eq!(failure2.test_name, "Calculator › handles divide by zero");
    assert_eq!(failure2.file.as_deref(), Some("src/calculator.ts"));
    assert_eq!(failure2.line, Some(8));
    assert!(failure2.error_message.contains("Division by zero"));
}

// ============================================================================
// Test 4: ANSI Stripping & Passing Test Outputs
// ============================================================================

#[test]
fn test_ansi_color_stripping() {
    let colored = "\x1b[31mFAIL\x1b[0m \x1b[1;33msrc/test.ts\x1b[0m";
    assert_eq!(strip_ansi(colored), "FAIL src/test.ts");
}

#[test]
fn test_passing_test_runs() {
    let passing_cargo =
        "test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
    let (passed, failures) = TddEngine::parse_test_output(passing_cargo, 0);
    assert!(passed);
    assert!(failures.is_empty());

    let passing_pytest = "========================= 5 passed in 0.10s ==========================";
    let (passed, failures) = TddEngine::parse_test_output(passing_pytest, 0);
    assert!(passed);
    assert!(failures.is_empty());

    let passing_vitest = "Tests  8 passed (8)";
    let (passed, failures) = TddEngine::parse_test_output(passing_vitest, 0);
    assert!(passed);
    assert!(failures.is_empty());
}

// ============================================================================
// Test 5: Phase State Transitions (Red -> Green -> Refactor -> Completed)
// ============================================================================

#[test]
fn test_phase_state_transitions() {
    let mut phase = TddPhase::Red;

    // 1. Red phase test fails -> remains Red
    phase = phase.next_phase(false);
    assert_eq!(phase, TddPhase::Red, "Failing test in Red must remain Red");

    // 2. Red phase test passes -> transitions to Green
    phase = phase.next_phase(true);
    assert_eq!(
        phase,
        TddPhase::Green,
        "Passing test in Red must transition to Green"
    );

    // 3. Green phase test passes -> transitions to Refactor
    phase = phase.next_phase(true);
    assert_eq!(
        phase,
        TddPhase::Refactor,
        "Passing test in Green advances to Refactor"
    );

    // 4. Refactor breaks test -> drops back to Red
    let broken_refactor = phase.next_phase(false);
    assert_eq!(
        broken_refactor,
        TddPhase::Red,
        "Broken test in Refactor reverts to Red"
    );

    // 5. Refactor test passes -> transitions to Completed
    phase = phase.next_phase(true);
    assert_eq!(
        phase,
        TddPhase::Completed,
        "Passing test in Refactor finishes as Completed"
    );
    assert!(phase.is_terminal());

    // 6. Completed phase remains Completed
    assert_eq!(phase.next_phase(true), TddPhase::Completed);
    assert_eq!(phase.next_phase(false), TddPhase::Completed);
}

// ============================================================================
// Test 6: Correction Guidance Generation
// ============================================================================

#[test]
fn test_correction_guidance_generation() {
    let failures = vec![
        TestFailure::new(
            "tests::test_add",
            "assertion `left == right` failed\n  left: 4\n right: 5",
        )
        .with_location("src/math.rs", 18),
        TestFailure::new("tests::test_zero_div", "attempt to divide by zero")
            .with_location("src/math.rs", 32)
            .with_stack_trace("at math::div (src/math.rs:32:5)"),
    ];

    let guidance = TddEngine::generate_correction_guidance(&failures);

    assert!(guidance.contains("## TDD Loop Diagnostic & Guidance"));
    assert!(guidance.contains("Detected 2 failing test(s)"));
    assert!(guidance.contains("Failure 1: `tests::test_add`"));
    assert!(guidance.contains("src/math.rs:18"));
    assert!(guidance.contains("Failure 2: `tests::test_zero_div`"));
    assert!(guidance.contains("src/math.rs:32"));
    assert!(guidance.contains("Zero division error"));
    assert!(guidance.contains("1. **Minimal Change Principle**"));
    assert!(guidance.contains("2. **Test Invariance**"));
    assert!(guidance.contains("3. **Regression Prevention**"));
}

#[test]
fn test_empty_correction_guidance() {
    let guidance = TddEngine::generate_correction_guidance(&[]);
    assert!(guidance.contains("All tests passed"));
}

// ============================================================================
// Test 7: TddEngine Iteration Tracking
// ============================================================================

#[test]
fn test_engine_iteration_tracking() {
    let mut engine = TddEngine::new();
    assert_eq!(engine.current_phase, TddPhase::Red);
    assert_eq!(engine.current_iteration, 0);

    // First iteration: Red test failure
    let iter1 = TddIteration::new(
        1,
        TddPhase::Red,
        "cargo test",
        vec![TestFailure::new("test_foo", "failed assertion")],
        false,
    );
    let next_phase = engine.record_iteration(iter1);
    assert_eq!(next_phase, TddPhase::Red);
    assert_eq!(engine.current_iteration, 1);
    assert_eq!(engine.history.len(), 1);

    // Second iteration: minimal fix, tests pass -> Green
    let iter2 = TddIteration::new(2, TddPhase::Red, "cargo test", vec![], true);
    let next_phase = engine.record_iteration(iter2);
    assert_eq!(next_phase, TddPhase::Green);
    assert_eq!(engine.current_iteration, 2);
    assert_eq!(engine.history.len(), 2);

    // Reset
    engine.reset();
    assert_eq!(engine.current_phase, TddPhase::Red);
    assert_eq!(engine.current_iteration, 0);
    assert!(engine.history.is_empty());
}
