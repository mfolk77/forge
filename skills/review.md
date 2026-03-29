# Skill: Code Review Checklist

**Trigger:** Use when reviewing code changes or pull requests.

## Checklist

### Correctness
- [ ] Does the code do what the PR description claims?
- [ ] Are edge cases handled (empty input, null, overflow, off-by-one)?
- [ ] Are error paths handled — no silent swallows, no panics in library code?
- [ ] Do new branches have matching tests?

### Security (P0 — never skip)
- [ ] Input validation on all external data (user input, API responses, file contents)
- [ ] No path traversal — reject `..` in file paths from untrusted sources
- [ ] No command injection — arguments to shell commands are escaped or avoided
- [ ] No SQL/NoSQL injection — parameterized queries only
- [ ] Auth checks on every protected endpoint or operation
- [ ] Secrets not hardcoded or logged

### Design
- [ ] Single responsibility — each function/method does one thing
- [ ] No unnecessary public API surface
- [ ] Error types are meaningful (not stringly-typed)
- [ ] No premature optimization — profile first

### Style & Maintainability
- [ ] Naming is clear and consistent with the codebase
- [ ] No dead code, commented-out blocks, or TODO without a ticket
- [ ] Complex logic has comments explaining *why*

### Tests
- [ ] New code has unit tests
- [ ] Security-sensitive code has red tests (adversarial inputs)
- [ ] Tests are deterministic — no flaky time-dependent or order-dependent tests

## How to Give Feedback

- Be specific: link to the exact line, suggest a concrete fix.
- Distinguish blocking issues from nits: prefix nits with "nit:" so the author knows.
- Ask questions instead of making accusations: "Could this overflow?" not "This is broken."
