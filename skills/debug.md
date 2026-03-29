# Skill: Systematic Debugging

**Trigger:** Use when diagnosing bugs, test failures, or unexpected behavior.

## Steps

1. **Reproduce** — get a minimal, reliable reproduction. If you cannot trigger the bug on demand, you cannot verify a fix.
2. **Isolate** — narrow the scope. Binary search through commits (`git bisect`), comment out code, add logging. Find the smallest input that triggers the bug.
3. **Hypothesize** — form a specific, falsifiable theory: "The bug occurs because X is null when Y calls Z."
4. **Verify the hypothesis** — add a targeted assertion, log, or test that confirms or refutes your theory. Do not guess-and-patch.
5. **Fix** — make the smallest change that addresses the root cause, not the symptom.
6. **Add a regression test** — write a test that fails without the fix and passes with it. Name it clearly (e.g., `test_issue_312_session_expiry_utc`).
7. **Check for siblings** — search for the same pattern elsewhere in the codebase. If the bug was a class of mistake, fix all instances.

## Debugging Tools

- `git bisect` — binary search through commits to find the introducing change.
- `RUST_LOG=debug` / `RUST_BACKTRACE=1` — get full traces in Rust.
- `println!` debugging — fast and effective. Remove before committing.
- Breakpoint debuggers (`lldb`, `gdb`, browser DevTools) — use for complex state.
- `strace` / `dtrace` — trace system calls when the bug is at the OS boundary.

## Common Traps

- Fixing symptoms instead of root causes (the bug will return in a different form).
- Debugging production without a local reproduction (too slow, too risky).
- Assuming the bug is in someone else's code — check yours first.
- Changing multiple things at once — you will not know which change fixed it.

## Security Considerations

- If the bug involves user input, check whether the same input could be exploited (injection, overflow, path traversal).
- Never leave debug logging of sensitive data (tokens, passwords, PII) in committed code.
