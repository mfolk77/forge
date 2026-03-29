# Skill: Refactoring Guide

**Trigger:** Use when restructuring or improving existing code without changing behavior.

## Steps

1. **Ensure test coverage first** — never refactor code that lacks tests. Write them before you start.
2. **Identify the smell** — duplicated logic, long functions, deep nesting, shotgun surgery, feature envy.
3. **Pick one refactoring at a time** — extract method, inline variable, replace conditional with polymorphism, etc.
4. **Make the change** — keep each commit to a single refactoring step.
5. **Run tests after every step** — if tests break, revert the last step and try a smaller change.
6. **Verify behavior is unchanged** — diff the test output before and after. No new failures, no removed assertions.

## Common Patterns

| Smell | Refactoring |
|---|---|
| Long function (>40 lines) | Extract method |
| Duplicated blocks | Extract shared function |
| Deep nesting (>3 levels) | Early return / guard clauses |
| Boolean parameters | Split into two functions |
| God struct/class | Extract responsibility into new type |
| Stringly-typed errors | Introduce error enum |

## Anti-Patterns to Avoid

- Refactoring and adding features in the same commit — separate them.
- Renaming across the whole codebase without grep-verifying all call sites.
- Changing public API signatures without updating all consumers.
- "Improving" code that works and is not in the change path — leave it alone.

## Security Considerations

- Refactoring auth or permission code requires extra scrutiny — re-run all security red tests.
- Moving validation logic must preserve the same validation guarantees at the new call site.
- Never weaken error handling during cleanup (e.g., replacing `Result` with `unwrap`).
