# Skill: Git Commit Best Practices

**Trigger:** Use when creating or advising on git commits.

## Steps

1. **Review the diff first** — run `git diff --staged` and read every hunk before writing a message.
2. **Stage selectively** — prefer `git add <file>` over `git add -A`. Never stage secrets, binaries, or generated files.
3. **Write a clear subject line** — imperative mood, under 72 characters, no trailing period. Example: `Add input validation to signup form`.
4. **Add a body when needed** — separate from subject with a blank line. Explain *why*, not *what* (the diff shows the what).
5. **Reference issues** — include `Fixes #123` or `Closes #456` when applicable.
6. **One logical change per commit** — split unrelated changes into separate commits.
7. **Verify before pushing** — run `cargo test` / `npm test` / relevant test suite after committing.

## Bad vs Good

```
# Bad
git add -A && git commit -m "stuff"

# Good
git add src/auth.rs tests/auth_test.rs
git commit -m "Fix session expiry check for refresh tokens

The previous logic compared timestamps in local time instead of UTC,
causing premature session expiry for users in negative UTC offsets.

Fixes #312"
```

## Security Considerations

- Never commit `.env`, credentials, API keys, or private keys.
- Check `git diff --staged` for accidental secret exposure before every commit.
- If secrets were committed, rotate them immediately — `git revert` does not erase history.
