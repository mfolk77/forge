# Skill: Test Writing Guide

**Trigger:** Use when writing or improving automated tests.

## Structure: Arrange-Act-Assert

```rust
#[test]
fn test_transfer_insufficient_funds() {
    // Arrange — set up preconditions
    let mut account = Account::new(100.0);

    // Act — perform the action under test
    let result = account.transfer(200.0);

    // Assert — verify the outcome
    assert!(result.is_err());
    assert_eq!(account.balance(), 100.0); // unchanged
}
```

## What to Test

1. **Happy path** — the normal, expected input produces correct output.
2. **Edge cases** — empty input, zero, max values, boundary conditions, Unicode, very long strings.
3. **Error paths** — invalid input returns the correct error, not a panic.
4. **Security red tests** — adversarial input: path traversal (`../`), injection (`; rm -rf`), oversized payloads.
5. **Integration points** — verify components work together, not just in isolation.

## Test Quality Checklist

- [ ] Each test has a descriptive name that explains the scenario and expected outcome.
- [ ] Tests are independent — no shared mutable state, no ordering dependencies.
- [ ] Tests are deterministic — no reliance on wall-clock time, random values, or network.
- [ ] Tests are fast — mock or stub slow dependencies (network, filesystem, databases).
- [ ] Assertions are specific — `assert_eq!(result, expected)` not just `assert!(result.is_ok())`.

## Coverage Goals

- Aim for high coverage on business logic and security-critical paths.
- Do not chase 100% line coverage — test behavior, not implementation details.
- Every bug fix should come with a regression test.

## Anti-Patterns

- Testing private implementation details — test the public interface instead.
- Snapshot tests for frequently changing output — they become noise.
- Tests that pass even when the feature is broken (vacuous assertions).
- Ignoring flaky tests — fix or delete them; a flaky test is worse than no test.
