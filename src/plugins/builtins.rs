use anyhow::{Context, Result};
use std::path::Path;

const FOLKTECH_PLUGIN_NAME: &str = "folktech-dev-toolkit";
const FOLKTECH_PLUGIN_VERSION: &str = "2.0.0";

const SUPERPOWERS_PLUGIN_NAME: &str = "forge-superpowers";
const SUPERPOWERS_PLUGIN_VERSION: &str = "1.0.0";

const PLUGIN_TOML: &str = r#"[plugin]
name = "folktech-dev-toolkit"
version = "2.0.0"
description = "Coherence gates, mental model checkpoints, TDD, dependency auditing, security, performance, and code quality tools"
author = "FolkTech AI"

[[skills]]
name = "secure-coding"
file = "skills/secure-coding/SKILL.md"
description = "FolkTech Secure Coding Standard — mandatory security red tests for all code"
trigger = "/secure"

[[skills]]
name = "tdd-patterns"
file = "skills/tdd-patterns/SKILL.md"
description = "Test-driven development patterns across multiple languages"
trigger = "/tdd"

[[skills]]
name = "perf-patterns"
file = "skills/perf-patterns/SKILL.md"
description = "Performance anti-pattern detection and fixes"
trigger = "/perf"

[[skills]]
name = "dependency-audit"
file = "skills/dependency-audit/SKILL.md"
description = "Audit packages for security, license, and maintenance issues"
trigger = "/audit-dep"

[[skills]]
name = "doc-patterns"
file = "skills/doc-patterns/SKILL.md"
description = "Documentation best practices — why not what"
trigger = "/doc"

[[skills]]
name = "code-quality"
file = "skills/code-quality/SKILL.md"
description = "Naming conventions, complexity thresholds, style guides"
trigger = "/quality"
"#;

const SKILL_SECURE_CODING: &str = r#"# FolkTech Secure Coding Standard

## Rule: No Code Ships Without Security Tests

Every code change — feature, fix, refactor — must include corresponding security tests.

## The Red Test Rule

For every piece of code written, ask:
1. What can an attacker send as input? → Write a test that sends it
2. What should NOT happen? → Assert it doesn't
3. What sensitive data could leak? → Write a test that checks

## Security Test Categories

### Category 1: Input Injection
Test vectors: SQL injection, path traversal, null bytes, unicode confusables, command injection, newline injection, HTML/script injection, max length strings, empty input

### Category 2: Path & File Security
Test vectors: parent traversal (../../), absolute system paths, sensitive directories (~/.ssh/, Keychains), symlink resolution, null bytes in paths
Rules: Always validate against allowlist, resolve symlinks, strip null bytes, blocklist .ssh/Keychains/etc/usr/var/System

### Category 3: Sensitive Data Exposure
Test vectors: password patterns, credit card numbers, SSNs, API keys, tokens, stack traces
Rules: Never log sensitive data, redact PII, no stack traces in user errors

### Category 4: Authentication & Authorization Bypass
Test vectors: storage tampering, hardcoded keys, spoofed responses, trial reuse, count resets

### Category 5: System Command Safety
Test vectors: string interpolation breakout, scheme injection, parameter confusion
Rules: Sanitize before interpolating, URL scheme allowlist (https/http only)

### Category 6: Cross-Platform Security
Additional vectors: Windows path separators, reserved names (CON/PRN/NUL), registry manipulation, DLL injection, PowerShell injection

## Workflow
1. Write functional code → 2. Evaluate against all 6 categories → 3. Write red tests → 4. Run (confirm fail) → 5. Implement fixes → 6. Run (confirm pass) → 7. Full test suite

## Naming Convention
test_[component]_[attackVector]_[expectedBehavior]

## Priority
P0 (NEVER skip): Input Injection, Path Security, Auth Bypass
P1 (within 48h): Data Exposure, Command Safety
P2 (within 2 weeks): Cross-Platform

## The One Rule
If you are unsure whether something is a security risk, it is. Write the test.
"#;

const SKILL_TDD_PATTERNS: &str = r#"# TDD Patterns

## Test Categories
Every test suite must cover:
1. Happy Path — verify correct behavior with valid inputs
2. Edge Cases — empty inputs, single elements, max/min values, unicode
3. Error Conditions — invalid types, out of range, missing params, IO failures
4. Boundary Values — exact boundaries where behavior changes

## Language Patterns

### Rust
- Use #[cfg(test)] mod tests { use super::*; }
- Naming: test_function_scenario_expected()
- Use assert!, assert_eq!, assert_ne!, matches!()
- #[should_panic(expected = "message")] for panic tests
- tempfile crate for filesystem tests

### Swift
- XCTest framework, func testMethodScenarioExpected()
- XCTAssertEqual, XCTAssertNil, XCTAssertThrowsError
- setUp()/tearDown() for test lifecycle

### TypeScript/JavaScript
- describe/it blocks, expect().toBe/toEqual/toThrow
- beforeEach/afterEach for setup
- jest.mock() for dependencies

### Python
- pytest preferred, def test_function_scenario_expected()
- assert statements, pytest.raises for exceptions
- @pytest.fixture for setup
"#;

const SKILL_PERF_PATTERNS: &str = r#"# Performance Patterns

## O(n²) Detection
- Nested loops over same collection → use HashMap/Set for O(1) lookup
- .contains()/.includes()/.indexOf() inside a loop → use HashSet
- String concatenation in a loop → use StringBuilder/join/push_str
- .filter() or .find() inside a loop → pre-index with HashMap

## Memory Leaks
- Swift: closures without [weak self] for timers, observers, async callbacks
- JavaScript: addEventListener without cleanup, setInterval without clearInterval
- Rust: Rc cycles without Weak, unbounded channels, leaked JoinHandles

## Main Thread Blocking
- Sync file I/O (readFileSync, Data(contentsOf:)) → move to background
- Heavy computation on main thread → dispatch to background queue/thread
- Blocking network calls → use async/await

## N+1 Queries
- Database query inside a loop → batch query with IN clause
- API call per item → batch endpoint or parallel with rate limiting
"#;

const SKILL_DEPENDENCY_AUDIT: &str = r#"# Dependency Audit

## 5 Audit Dimensions
1. Security — Check CVE databases (NVD, Snyk, GitHub Advisory)
2. License — Permissive (MIT, Apache, BSD) OK; Copyleft (GPL, AGPL) WARN
3. Maintenance — Last commit, open issues ratio, bus factor
4. Size — Bundle size, dependency tree depth
5. Alternatives — Standard library alternatives, lighter options

## Severity Actions
- Critical (RCE, auth bypass): REJECT
- High (SQLi, XSS): WARN strongly
- Medium (DoS, info disclosure): WARN
- Low (theoretical): NOTE
"#;

const SKILL_DOC_PATTERNS: &str = r#"# Documentation Patterns

## Core Principle: Why Not What
Bad: "Increment i by 1" — reader can see that
Good: "Skip header row which contains column names, not data"

## When to Document
ALWAYS: Complex algorithms, business logic, workarounds, perf optimizations, security considerations, magic numbers, regex patterns
NEVER: Obvious getter/setter behavior, type info already in signature, code that is self-documenting

## Format by Language
- Rust: /// doc comments with # Examples section
- Swift: /// with - Parameter:, - Returns:, - Throws:
- TypeScript: /** JSDoc */ with @param, @returns, @throws
- Python: Google-style docstrings with Args:, Returns:, Raises:
"#;

const SKILL_CODE_QUALITY: &str = r#"# Code Quality Patterns

## Naming Conventions
- Rust: snake_case (vars/fns), PascalCase (types), SCREAMING_SNAKE (constants)
- Swift: lowerCamelCase (vars/fns), PascalCase (types), protocols often -able/-ible
- TypeScript: camelCase (vars/fns), PascalCase (classes/interfaces), kebab-case (files)
- Python: snake_case (vars/fns), PascalCase (classes), SCREAMING_SNAKE (constants)

## Complexity Thresholds
- Cyclomatic complexity: ≤10 per function (warn at 7)
- Nesting depth: ≤4 levels
- Function length: ≤50 lines (warn at 30)
- Parameters: ≤5 per function
- File length: ≤500 lines (warn at 300)

## Code Smells
- Long parameter lists → use struct/object
- Deep nesting → extract early returns/guard clauses
- Duplicate code (3+ occurrences) → extract function
- God objects (>10 responsibilities) → split into focused types
- Boolean parameters → use enum for clarity
"#;

/// Skill definition for scaffolding: directory name and content.
struct SkillSpec {
    dir_name: &'static str,
    content: &'static str,
}

const SKILLS: &[SkillSpec] = &[
    SkillSpec {
        dir_name: "secure-coding",
        content: SKILL_SECURE_CODING,
    },
    SkillSpec {
        dir_name: "tdd-patterns",
        content: SKILL_TDD_PATTERNS,
    },
    SkillSpec {
        dir_name: "perf-patterns",
        content: SKILL_PERF_PATTERNS,
    },
    SkillSpec {
        dir_name: "dependency-audit",
        content: SKILL_DEPENDENCY_AUDIT,
    },
    SkillSpec {
        dir_name: "doc-patterns",
        content: SKILL_DOC_PATTERNS,
    },
    SkillSpec {
        dir_name: "code-quality",
        content: SKILL_CODE_QUALITY,
    },
];

// ---------------------------------------------------------------------------
// forge-superpowers plugin
// ---------------------------------------------------------------------------

const SUPERPOWERS_PLUGIN_TOML: &str = r#"[plugin]
name = "forge-superpowers"
version = "1.0.0"
description = "Workflow skills: brainstorming, planning, TDD, debugging, code review, parallel agents, verification"
author = "FolkTech AI"

[[skills]]
name = "brainstorming"
file = "skills/brainstorming/SKILL.md"
description = "Explore intent, requirements, and design before implementation"
trigger = "/brainstorm"

[[skills]]
name = "writing-plans"
file = "skills/writing-plans/SKILL.md"
description = "Design implementation plans for multi-step tasks before touching code"
trigger = "/plan"

[[skills]]
name = "executing-plans"
file = "skills/executing-plans/SKILL.md"
description = "Execute implementation plans with review checkpoints"
trigger = "/execute"

[[skills]]
name = "test-driven-development"
file = "skills/test-driven-development/SKILL.md"
description = "Write tests before implementation code"
trigger = "/tdd-workflow"

[[skills]]
name = "systematic-debugging"
file = "skills/systematic-debugging/SKILL.md"
description = "Diagnose bugs methodically — reproduce, isolate, fix, verify"
trigger = "/debug"

[[skills]]
name = "requesting-code-review"
file = "skills/requesting-code-review/SKILL.md"
description = "Review completed work against requirements before merging"
trigger = "/review"

[[skills]]
name = "receiving-code-review"
file = "skills/receiving-code-review/SKILL.md"
description = "Handle review feedback with technical rigor, not blind agreement"
trigger = "/review-feedback"

[[skills]]
name = "dispatching-parallel-agents"
file = "skills/dispatching-parallel-agents/SKILL.md"
description = "Split independent tasks across parallel subagents"
trigger = "/parallel"

[[skills]]
name = "subagent-driven-development"
file = "skills/subagent-driven-development/SKILL.md"
description = "Execute plans with independent subagent tasks"
trigger = "/subagent-dev"

[[skills]]
name = "verification-before-completion"
file = "skills/verification-before-completion/SKILL.md"
description = "Run verification commands before claiming work is done"
trigger = "/verify"

[[skills]]
name = "finishing-a-development-branch"
file = "skills/finishing-a-development-branch/SKILL.md"
description = "Guide completion of development work — merge, PR, or cleanup"
trigger = "/finish"
"#;

const SP_SKILL_BRAINSTORMING: &str = r#"# Brainstorming

Before implementing any feature, explore the problem space.

## Process
1. **Clarify intent** — What is the user actually trying to achieve? Ask if unclear.
2. **Identify constraints** — Performance, compatibility, existing patterns, deadlines.
3. **List approaches** — At least 2-3 valid approaches with trade-offs.
4. **Evaluate** — Score each approach on: complexity, maintainability, performance, risk.
5. **Recommend** — Pick one and explain why. Get user confirmation before proceeding.

## When to brainstorm
- New features with multiple valid approaches
- Architectural decisions that affect multiple files
- Performance-sensitive code with trade-offs
- Anything where "just do it" risks wasted work

## Output format
Present as a brief analysis:
- Problem: [1 sentence]
- Approaches: [2-3 options with trade-offs]
- Recommendation: [chosen approach + why]
- Risks: [what could go wrong]
"#;

const SP_SKILL_WRITING_PLANS: &str = r#"# Writing Implementation Plans

Create step-by-step plans before multi-step tasks.

## When to plan
- Task requires changes to 3+ files
- Task has dependencies between steps
- Task involves unfamiliar code areas
- User explicitly asks for a plan

## Plan format
1. **Goal** — What we're building and why (1-2 sentences)
2. **Steps** — Numbered, each with:
   - What to do
   - Which file(s) to modify/create
   - What to test after this step
3. **Dependencies** — Which steps block others
4. **Risks** — What might go wrong, fallback approaches
5. **Verification** — How to confirm the whole thing works

## Rules
- Each step should be independently testable
- Steps should be ordered to minimize risk (easy/safe first)
- Include "checkpoint" steps where you verify before continuing
- Plan should fit in one screen (if longer, break into phases)
"#;

const SP_SKILL_EXECUTING_PLANS: &str = r#"# Executing Implementation Plans

Follow plans methodically with review checkpoints.

## Process
1. Read the plan fully before starting
2. For each step:
   a. Announce what you're about to do
   b. Do it
   c. Run the step's test/verification
   d. If it fails — diagnose before moving on
   e. Mark step complete
3. After all steps: run full verification
4. Report what was done, what was tested, any deviations

## Checkpoint rules
- After every 3rd step, pause for review
- After any step that modifies >50 lines, pause for review
- If you deviate from the plan, explain why before proceeding
- Never skip verification steps

## Deviation handling
If the plan needs adjustment mid-execution:
1. Explain what changed and why
2. Propose the adjusted steps
3. Get confirmation before continuing
"#;

const SP_SKILL_TDD: &str = r#"# TDD Workflow

Write tests BEFORE implementation. Red → Green → Refactor.

## Process
1. **Red** — Write a failing test that defines desired behavior
2. **Green** — Write the minimum code to make the test pass
3. **Refactor** — Clean up while keeping tests green

## What to test first
1. Happy path (expected inputs → expected outputs)
2. Edge cases (empty, nil, max, min, boundary)
3. Error cases (invalid input, missing data, network failure)
4. Security cases (injection, traversal, overflow)

## Rules
- Never write implementation before a test exists for it
- Each test should test ONE thing
- Test names describe the scenario: test_function_scenario_expected()
- If you can't write a test, the requirement isn't clear enough — ask

## When TDD applies
- New functions/methods
- Bug fixes (write test that reproduces bug first)
- Refactors (ensure tests exist before changing code)
- NOT for: config files, UI layout, one-off scripts
"#;

const SP_SKILL_DEBUGGING: &str = r#"# Systematic Debugging

Diagnose bugs methodically. Never guess-and-check.

## Process
1. **Reproduce** — Get the exact error message, stack trace, or behavior
2. **Isolate** — Narrow down to the smallest reproducing case
3. **Hypothesize** — Form a theory about the root cause (not symptoms)
4. **Verify** — Test the hypothesis with targeted investigation
5. **Fix** — Address root cause, not symptoms
6. **Confirm** — Run tests, verify the fix, check for regressions

## Anti-patterns to avoid
- Changing random things hoping something works
- Fixing symptoms without understanding root cause
- Adding error handling to hide the real problem
- Assuming the bug is in the last thing you changed

## Investigation tools
- Read the actual error message carefully
- Check git blame / recent changes to affected code
- Add targeted logging (not shotgun logging)
- Bisect: find the exact commit that introduced the bug
- Simplify: remove code until the bug disappears, then add back

## Output
After fixing, always report:
- Root cause (1 sentence)
- Fix applied (what changed)
- How verified (what test proves it's fixed)
"#;

const SP_SKILL_REQUESTING_REVIEW: &str = r#"# Requesting Code Review

Review completed work against requirements before merging.

## Checklist
1. **Requirements met** — Does the code do what was asked?
2. **Tests pass** — Run full test suite, not just new tests
3. **No regressions** — Existing functionality still works
4. **Code quality** — Naming, structure, complexity reasonable
5. **Security** — Input validation, auth checks, no secrets in code
6. **Edge cases** — Empty inputs, large inputs, concurrent access

## Review output
- Files changed: [list with brief description]
- Tests: [count passing / count total]
- Areas needing attention: [specific concerns]
- Confidence: [high/medium/low] with reasoning
"#;

const SP_SKILL_RECEIVING_REVIEW: &str = r#"# Receiving Code Review Feedback

Handle review feedback with technical rigor.

## Rules
1. **Don't blindly agree** — Verify feedback is technically correct
2. **Don't performatively apologize** — Just fix the issue
3. **Push back when warranted** — If feedback is wrong, explain why with evidence
4. **Ask for clarification** — If feedback is vague, ask for specific examples
5. **Test after changes** — Every fix from review feedback must be verified

## Process
1. Read all feedback before making changes
2. For each item: verify it's correct, then fix or discuss
3. Run tests after all changes
4. Report what was changed and what was pushed back on
"#;

const SP_SKILL_PARALLEL_AGENTS: &str = r#"# Dispatching Parallel Agents

Split independent tasks across subagents for speed.

## When to parallelize
- 2+ tasks with NO shared state or dependencies
- Each task is self-contained (doesn't need results from another)
- Tasks operate on different files/directories

## When NOT to parallelize
- Tasks depend on each other's output
- Tasks modify the same files
- Order matters (migrations, schema changes)

## Process
1. Identify independent tasks
2. Write clear, complete prompts for each (they have no conversation context)
3. Launch all in parallel
4. Wait for results
5. Integrate — check for conflicts, run tests

## Agent prompt rules
- Include full context (the agent knows nothing)
- Specify exact files to read/modify
- State expected output format
- Say whether to write code or just research
"#;

const SP_SKILL_SUBAGENT_DEV: &str = r#"# Subagent-Driven Development

Execute plans by dispatching steps to focused subagents.

## When to use
- Plan has 3+ independent steps
- Steps touch different areas of the codebase
- You want to parallelize implementation

## Process
1. Break plan into independent work units
2. For each unit, create a focused prompt with:
   - What to build
   - Which files to modify
   - What tests to write
   - What constraints to follow
3. Launch independent units in parallel
4. Sequential units run after their dependencies complete
5. After all complete: integration test

## Rules
- Each agent gets a complete, self-contained brief
- Never assume agents share context
- Review each agent's output before integration
- Run full test suite after merging all results
"#;

const SP_SKILL_VERIFICATION: &str = r#"# Verification Before Completion

Never claim work is done without evidence.

## Before saying "done":
1. Run the build — does it compile/pass lint?
2. Run the tests — ALL tests, not just new ones
3. Run the specific verification for this task
4. Check for regressions — did anything else break?

## Evidence required
- Build output (success)
- Test output (count passing, count failing)
- Manual verification result if applicable

## Rules
- "I believe it works" is not evidence
- "It should work" is not evidence
- Only actual command output counts
- If verification fails, fix it before claiming done
"#;

const SP_SKILL_FINISHING_BRANCH: &str = r#"# Finishing a Development Branch

Guide completion of feature work.

## Checklist before merge/PR:
1. All tests pass (full suite, not just new)
2. No uncommitted changes
3. Branch is up to date with base branch
4. Commit messages are clean and descriptive
5. No temporary debug code, console.log, TODO hacks

## Options
- **Merge directly** — Small changes, single author, low risk
- **Create PR** — Needs review, multiple stakeholders, risky changes
- **Squash and merge** — Many small commits that should be one logical change

## PR description
- What changed (2-3 sentences)
- Why (motivation/ticket)
- How to test
- Any risks or follow-up needed
"#;

const SUPERPOWERS_SKILLS: &[SkillSpec] = &[
    SkillSpec {
        dir_name: "brainstorming",
        content: SP_SKILL_BRAINSTORMING,
    },
    SkillSpec {
        dir_name: "writing-plans",
        content: SP_SKILL_WRITING_PLANS,
    },
    SkillSpec {
        dir_name: "executing-plans",
        content: SP_SKILL_EXECUTING_PLANS,
    },
    SkillSpec {
        dir_name: "test-driven-development",
        content: SP_SKILL_TDD,
    },
    SkillSpec {
        dir_name: "systematic-debugging",
        content: SP_SKILL_DEBUGGING,
    },
    SkillSpec {
        dir_name: "requesting-code-review",
        content: SP_SKILL_REQUESTING_REVIEW,
    },
    SkillSpec {
        dir_name: "receiving-code-review",
        content: SP_SKILL_RECEIVING_REVIEW,
    },
    SkillSpec {
        dir_name: "dispatching-parallel-agents",
        content: SP_SKILL_PARALLEL_AGENTS,
    },
    SkillSpec {
        dir_name: "subagent-driven-development",
        content: SP_SKILL_SUBAGENT_DEV,
    },
    SkillSpec {
        dir_name: "verification-before-completion",
        content: SP_SKILL_VERIFICATION,
    },
    SkillSpec {
        dir_name: "finishing-a-development-branch",
        content: SP_SKILL_FINISHING_BRANCH,
    },
];

// ---------------------------------------------------------------------------
// Scaffolding helpers
// ---------------------------------------------------------------------------

/// Scaffold a single built-in plugin given its name, version, TOML, and skills.
/// Returns `Ok(true)` if created, `Ok(false)` if already present.
// ---------------------------------------------------------------------------
// forge-dev-tools plugin
// ---------------------------------------------------------------------------

const DEV_TOOLS_NAME: &str = "forge-dev-tools";
const DEV_TOOLS_VERSION: &str = "1.0.0";

const DEV_TOOLS_TOML: &str = r#"[plugin]
name = "forge-dev-tools"
version = "1.0.0"
description = "Development utilities: changelog generation, frontend design, file organization, project management"
author = "FolkTech AI"

[[skills]]
name = "changelog-generator"
file = "skills/changelog-generator/SKILL.md"
description = "Generate user-facing changelogs from git history"
trigger = "/changelog"

[[skills]]
name = "frontend-design"
file = "skills/frontend-design/SKILL.md"
description = "Create production-grade frontend interfaces with high design quality"
trigger = "/frontend"

[[skills]]
name = "file-organizer"
file = "skills/file-organizer/SKILL.md"
description = "Organize files and directories by context and purpose"
trigger = "/organize"

[[skills]]
name = "content-research-writer"
file = "skills/content-research-writer/SKILL.md"
description = "Research topics and write well-cited content"
trigger = "/research"

[[skills]]
name = "webapp-testing"
file = "skills/webapp-testing/SKILL.md"
description = "Test web applications using browser automation"
trigger = "/webapp-test"

[[skills]]
name = "mcp-builder"
file = "skills/mcp-builder/SKILL.md"
description = "Build Model Context Protocol servers for tool integrations"
trigger = "/mcp"

[[skills]]
name = "skill-creator"
file = "skills/skill-creator/SKILL.md"
description = "Create and improve Forge skills"
trigger = "/create-skill"

[[skills]]
name = "config-auditor"
file = "skills/config-auditor/SKILL.md"
description = "Audit and improve FTAI.md project config files"
trigger = "/audit-config"

[[skills]]
name = "image-enhancer"
file = "skills/image-enhancer/SKILL.md"
description = "Enhance image quality for documentation and presentations"
trigger = "/enhance-image"

[[skills]]
name = "internal-comms"
file = "skills/internal-comms/SKILL.md"
description = "Write status reports, leadership updates, and team communications"
trigger = "/comms"
"#;

const DEV_TOOLS_SKILLS: &[SkillSpec] = &[
    SkillSpec { dir_name: "changelog-generator", content: "# Changelog Generator\n\nGenerate user-facing changelogs from git commit history.\n\n## Process\n1. Run `git log --oneline` for the target range\n2. Categorize: Features, Fixes, Breaking Changes, Improvements, Docs\n3. Rewrite technical commits into user-friendly descriptions\n4. Group by category, order by impact\n\n## Format\n### [version] - YYYY-MM-DD\n#### Added\n- Description of new feature\n\n#### Fixed\n- Description of bug fix\n\n#### Breaking\n- What broke and migration steps\n\n## Rules\n- Skip refactor commits unless they affect behavior\n- Combine related commits into single entries\n- Use present tense\n- Include PR/issue links when available\n" },
    SkillSpec { dir_name: "frontend-design", content: "# Frontend Design\n\nCreate distinctive, production-grade frontend interfaces.\n\n## Design principles\n- Mobile-first responsive design\n- Consistent spacing scale (4px base)\n- Color hierarchy: primary, secondary, destructive, neutral\n- Typography scale: 12-40px\n\n## Component patterns\n- Cards: rounded corners, subtle shadow, consistent padding\n- Forms: labels above inputs, clear error states\n- Navigation: sticky header, breadcrumbs, active states\n- Tables: sortable headers, row hover, pagination\n- Modals: overlay, close on Esc, focus trap\n\n## Accessibility\n- Keyboard-navigable interactive elements\n- Color contrast >= 4.5:1\n- ARIA labels on icon-only buttons\n- Focus indicators visible\n\n## Avoid\n- Generic AI aesthetics (gradients, over-rounded)\n- Placeholder text as labels\n- Layout shifts on content load\n" },
    SkillSpec { dir_name: "file-organizer", content: "# File Organizer\n\nOrganize files and directories by context and purpose.\n\n## Analysis\n1. Scan for file types and patterns\n2. Identify logical groups (feature, type, domain)\n3. Detect duplicates\n4. Suggest structure improvements\n\n## Patterns\n- By feature: group component + test + style\n- By type: models/, views/, controllers/\n- By domain: users/, orders/, payments/\n\n## Rules\n- Never delete without confirmation\n- Show before/after comparison\n- Preserve git history (git mv)\n- Handle naming conflicts\n" },
    SkillSpec { dir_name: "content-research-writer", content: "# Content Research Writer\n\nResearch topics and write well-cited content.\n\n## Process\n1. Clarify topic and audience\n2. Search authoritative sources\n3. Cross-verify claims\n4. Write with citations\n5. Review for accuracy\n\n## Rules\n- Lead with most important information\n- Use concrete examples\n- Cite sources for factual claims\n- Distinguish facts from opinions\n- Include code examples where relevant\n" },
    SkillSpec { dir_name: "webapp-testing", content: "# Web Application Testing\n\nTest web apps using browser automation.\n\n## Test types\n- Smoke: app loads, key pages render\n- Functional: forms submit, data saves\n- Visual: layout correct, responsive\n- Performance: load time, no memory leaks\n\n## Tools\n- Playwright preferred (cross-browser, auto-wait)\n- Screenshot comparison for visual regression\n- Network interception for API mocking\n\n## Best practices\n- Wait for elements, not fixed sleeps\n- Use data-testid for selectors\n- Clean up test data\n- Headless in CI\n" },
    SkillSpec { dir_name: "mcp-builder", content: "# MCP Server Builder\n\nBuild Model Context Protocol servers.\n\n## Structure\n- Define tools with name, description, input schema\n- Define resources for data access\n- Handle tool calls and return results\n- Support streaming for long operations\n\n## Patterns\n- Python: FastMCP\n- TypeScript: @modelcontextprotocol/sdk\n- Rust: JSON-RPC directly\n\n## Best practices\n- Validate all inputs\n- Clear error messages, not stack traces\n- Document every tool with examples\n- Rate limit external API calls\n- Handle timeouts\n" },
    SkillSpec { dir_name: "skill-creator", content: "# Skill Creator\n\nCreate and improve Forge skills.\n\n## Structure\nskills/<name>/SKILL.md with optional YAML frontmatter.\n\n## Good descriptions\n- Describe WHEN to activate, not just what it does\n- Include trigger words users might say\n- Be specific about activation conditions\n\n## Good content\n- Core principle (1-2 sentences)\n- Step-by-step process\n- Concrete examples\n- Keep under 2000 tokens\n\n## Anti-patterns\n- Skills that are just documentation\n- Too broad to be useful\n- Duplicate built-in behavior\n" },
    SkillSpec { dir_name: "config-auditor", content: "# Config Auditor\n\nAudit FTAI.md project config files.\n\n## Checks\n1. Build commands — accurate and runnable?\n2. Architecture — reflects current structure?\n3. Gotchas — known issues documented?\n4. Test commands — work correctly?\n5. Conventions — documented?\n\n## Quality scoring\n- Build commands: +20\n- Test commands: +20\n- Architecture: +15\n- Gotchas: +15\n- Conventions: +15\n- Up to date: +15\n\n## Auto-fix\n- Update test counts\n- Add missing file entries\n- Flag stale references\n" },
    SkillSpec { dir_name: "image-enhancer", content: "# Image Enhancer\n\nEnhance image quality for docs and presentations.\n\n## Capabilities\n- Upscale low-res screenshots\n- Adjust brightness/contrast\n- Crop to relevant content\n- Add annotations\n- Convert formats (PNG, JPEG, WebP, SVG)\n\n## For documentation\n- PNG for screenshots (lossless)\n- SVG for diagrams (scalable)\n- WebP for web (small size)\n- 2x resolution for retina\n" },
    SkillSpec { dir_name: "internal-comms", content: "# Internal Communications\n\nWrite status reports and team communications.\n\n## Status report\n- Progress: what was completed\n- Next: what's coming\n- Blockers: what needs help\n- Metrics: key numbers\n\n## Leadership update\n- Lead with business impact\n- Use concrete metrics\n- Highlight risks with mitigations\n- Max 5 bullet points\n\n## Team communication\n- Be direct and specific\n- Include action items with owners/dates\n- Separate FYI from action-required\n" },
];

// ---------------------------------------------------------------------------
// forge-document-tools plugin
// ---------------------------------------------------------------------------

const DOC_TOOLS_NAME: &str = "forge-document-tools";
const DOC_TOOLS_VERSION: &str = "1.0.0";

const DOC_TOOLS_TOML: &str = r#"[plugin]
name = "forge-document-tools"
version = "1.0.0"
description = "Document creation: PDF, DOCX, XLSX, PPTX, JSON Canvas, Markdown"
author = "FolkTech AI"

[[skills]]
name = "pdf"
file = "skills/pdf/SKILL.md"
description = "Create, read, merge, split, and fill PDF documents"
trigger = "/pdf"

[[skills]]
name = "docx"
file = "skills/docx/SKILL.md"
description = "Create and edit Word documents with formatting and tables"
trigger = "/docx"

[[skills]]
name = "xlsx"
file = "skills/xlsx/SKILL.md"
description = "Create spreadsheets with formulas, charts, and data analysis"
trigger = "/xlsx"

[[skills]]
name = "pptx"
file = "skills/pptx/SKILL.md"
description = "Create presentations with layouts, speaker notes, and themes"
trigger = "/pptx"

[[skills]]
name = "json-canvas"
file = "skills/json-canvas/SKILL.md"
description = "Create JSON Canvas files with nodes, edges, and visual layouts"
trigger = "/canvas"

[[skills]]
name = "markdown-formatter"
file = "skills/markdown-formatter/SKILL.md"
description = "Format and fix markdown syntax, headings, lists, and code blocks"
trigger = "/markdown"
"#;

const DOC_TOOLS_SKILLS: &[SkillSpec] = &[
    SkillSpec { dir_name: "pdf", content: "# PDF Operations\n\nCreate, read, merge, split, and fill PDF documents.\n\n## Create\n- Use PDF library (reportlab/Python, pdf-lib/JS, printpdf/Rust)\n- Set metadata: title, author, date\n- Consistent fonts and spacing\n- Include page numbers\n\n## Read/Extract\n- Extract text preserving structure\n- Extract tables into structured data\n\n## Merge/Split\n- Combine PDFs, preserve bookmarks\n- Extract specific page ranges\n\n## Form filling\n- Parse form field names\n- Fill programmatically\n- Flatten to prevent editing\n" },
    SkillSpec { dir_name: "docx", content: "# Word Document Creation\n\nCreate .docx files with professional formatting.\n\n## Structure\n- Title page, TOC, sections with heading hierarchy\n- Page headers/footers with numbers\n\n## Formatting\n- Use styles, not inline formatting\n- 1.15 line spacing body, 1.0 headings\n- 1 inch margins\n\n## Tables\n- Bold header row with shading\n- Alternating row colors\n- Proportional column widths\n" },
    SkillSpec { dir_name: "xlsx", content: "# Spreadsheet Creation\n\nCreate .xlsx with formulas and formatting.\n\n## Structure\n- Headers: bold, frozen first row\n- Correct data types: numbers, dates, currency\n- Named ranges for formulas\n\n## Formulas\n- SUM, AVERAGE, COUNT for basics\n- VLOOKUP/INDEX-MATCH for lookups\n- IF/SUMIF for conditional logic\n\n## Formatting\n- Number formats: #,##0 / #,##0.00\n- Conditional formatting for indicators\n- Data validation for dropdowns\n" },
    SkillSpec { dir_name: "pptx", content: "# Presentation Creation\n\nCreate .pptx with professional layouts.\n\n## Slide types\n- Title: title + subtitle + date\n- Content: heading + max 6 bullets\n- Two-column: comparison\n- Image: full-bleed + caption\n- Data: chart/table + takeaway\n\n## Design rules\n- One idea per slide\n- Max 6 bullets, max 6 words each\n- Font: 28+ body, 36+ titles\n- Consistent color palette\n\n## Speaker notes\n- Talking points, not scripts\n- Key data and sources\n- Transition phrases\n" },
    SkillSpec { dir_name: "json-canvas", content: "# JSON Canvas\n\nCreate .canvas files for visual knowledge mapping.\n\n## Structure\n{\"nodes\": [{\"id\", \"type\", \"text\", \"x\", \"y\", \"width\", \"height\"}], \"edges\": [{\"id\", \"fromNode\", \"toNode\"}]}\n\n## Node types\n- text: markdown content\n- file: link to a file\n- link: external URL\n- group: visual container\n\n## Layout patterns\n- Flow chart: top-to-bottom\n- Mind map: center + radial\n- Timeline: horizontal\n- Grid: organized rows/columns\n\n## Best practices\n- Space nodes 50px+ apart\n- Use groups for related nodes\n- Color-code by category\n" },
    SkillSpec { dir_name: "markdown-formatter", content: "# Markdown Formatter\n\nFix and standardize markdown formatting.\n\n## Checks\n- Heading hierarchy (no skipping levels)\n- Consistent list markers\n- Code block language tags\n- Link validity\n- Trailing whitespace\n\n## Fixes\n- Tabs to spaces\n- ATX headings (# style)\n- Fix nested list indentation\n- Add language to code blocks\n- Convert HTML to markdown\n\n## Rules\n- ATX headings preferred\n- One blank line before headings\n- No trailing spaces\n- End file with single newline\n- Consistent emphasis (* not _)\n" },
];

// ---------------------------------------------------------------------------
// forge-plugin-dev plugin
// ---------------------------------------------------------------------------

const PLUGIN_DEV_NAME: &str = "forge-plugin-dev";
const PLUGIN_DEV_VERSION: &str = "1.0.0";

const PLUGIN_DEV_TOML: &str = r#"[plugin]
name = "forge-plugin-dev"
version = "1.0.0"
description = "Tools for creating Forge plugins: structure, skills, hooks, commands, agents"
author = "FolkTech AI"

[[skills]]
name = "plugin-structure"
file = "skills/plugin-structure/SKILL.md"
description = "Create and scaffold Forge plugins with correct directory structure"
trigger = "/plugin-dev"

[[skills]]
name = "skill-development"
file = "skills/skill-development/SKILL.md"
description = "Create effective skills with good descriptions"
trigger = "/skill-dev"

[[skills]]
name = "hook-development"
file = "skills/hook-development/SKILL.md"
description = "Create pre/post tool hooks for validation and automation"
trigger = "/hook-dev"

[[skills]]
name = "command-development"
file = "skills/command-development/SKILL.md"
description = "Create slash commands with arguments"
trigger = "/command-dev"

[[skills]]
name = "agent-development"
file = "skills/agent-development/SKILL.md"
description = "Create specialized subagents with focused tool access"
trigger = "/agent-dev"
"#;

const PLUGIN_DEV_SKILLS: &[SkillSpec] = &[
    SkillSpec { dir_name: "plugin-structure", content: "# Forge Plugin Structure\n\nCreate plugins with the correct layout.\n\n## Directory structure\nmy-plugin/\n  plugin.toml — manifest\n  skills/ — SKILL.md files\n  hooks/ — hook scripts\n  tools/ — custom tool scripts\n  commands/ — slash commands\n  agents/ — subagent definitions\n  rules.ftai — plugin rules\n\n## plugin.toml\n[plugin]\nname, version, description, author\n[[skills]] name, file, description, trigger\n[[tools]] name, description, command\n[[hooks]] event, command\n\n## Naming\n- Alphanumeric + hyphens/underscores\n- No dots, spaces, path separators\n\n## Distribution\n- Install from git: forge plugin install <url>\n- List in catalog for discovery\n- Share via marketplace repos\n" },
    SkillSpec { dir_name: "skill-development", content: "# Skill Development\n\nCreate effective Forge skills.\n\n## Format\nskills/<name>/SKILL.md with optional YAML frontmatter.\n\n## Good descriptions\n- Describe WHEN to activate\n- Include trigger words\n- Be specific about conditions\n\n## Good content\n1. Core principle (1-2 sentences)\n2. Step-by-step process\n3. Concrete examples with code\n4. Common mistakes\n5. Under 2000 tokens\n\n## Progressive disclosure\n- Metadata loads at session start (~100 tokens)\n- Full content loads only on trigger\n- Many skills without bloating context\n" },
    SkillSpec { dir_name: "hook-development", content: "# Hook Development\n\nCreate pre/post tool hooks.\n\n## Types\n- PreToolUse: before tool, can block\n- PostToolUse: after tool, inspect results\n\n## In plugin.toml\n[[hooks]]\nevent = \"pre:file_write\"\ncommand = \"hooks/validate.sh\"\ntimeout_ms = 5000\n\n## Environment variables\n- FORGE_PROJECT, FORGE_TOOL_NAME\n- FORGE_TOOL_ARGS, FORGE_FILE_PATH\n\n## Exit codes\n- 0 = allow, non-zero = block (pre-hooks)\n- Stderr shown to user on block\n\n## Prompt-based hooks\nUse prompt string instead of shell script.\n\n## Best practices\n- Keep fast (<5s)\n- Advisory, not annoying\n- Silent on success (post-hooks)\n- Always set timeout\n" },
    SkillSpec { dir_name: "command-development", content: "# Command Development\n\nCreate slash commands for Forge.\n\n## Format\ncommands/<name>.md with YAML frontmatter:\n---\ndescription: what it does\nargument-hint: \"<arg> [optional]\"\nallowed-tools: [\"Read\", \"Grep\"]\n---\n\n## Body\nMarkdown prompt template. Tells AI how to execute.\n\n## Arguments\n- Available as $ARGUMENTS\n- Parse with positions: $1, $2\n- Validate required args early\n\n## Best practices\n- One purpose per command\n- Minimal allowed-tools\n- Include examples\n- Handle missing arguments\n" },
    SkillSpec { dir_name: "agent-development", content: "# Agent Development\n\nCreate specialized subagents.\n\n## Format\nagents/<name>.md with YAML frontmatter:\n---\nname: agent-name\ndescription: when to use\ntools: [Read, Grep, Write]\n---\n\n## Body = agent's system prompt\nDefines personality, capabilities, constraints.\n\n## Design principles\n- ONE clear purpose per agent\n- Restrict tools to minimum needed\n- Fresh context (no conversation history)\n- Returns summary, not raw output\n\n## When to create\n- Repetitive structured tasks\n- Different mindset needed (security auditor vs feature dev)\n- Benefits from context isolation\n\n## When NOT to\n- Simple one-off tasks\n- Needs conversation context\n- Overhead exceeds benefit\n" },
];

fn scaffold_builtin_plugin(
    plugins_dir: &Path,
    name: &str,
    version: &str,
    toml_content: &str,
    skills: &[SkillSpec],
) -> Result<bool> {
    let plugin_dir = plugins_dir.join(name);
    let manifest_path = plugin_dir.join("plugin.toml");

    if manifest_path.exists() {
        return Ok(false);
    }

    // Defense-in-depth: verify the name has no traversal characters.
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        anyhow::bail!("Built-in plugin name contains path traversal characters");
    }

    std::fs::create_dir_all(&plugin_dir)
        .with_context(|| format!("Failed to create plugin directory: {}", plugin_dir.display()))?;

    std::fs::write(&manifest_path, toml_content)
        .with_context(|| format!("Failed to write {}", manifest_path.display()))?;

    let version_path = plugin_dir.join(".version");
    std::fs::write(&version_path, version)
        .with_context(|| format!("Failed to write {}", version_path.display()))?;

    for skill in skills {
        let skill_dir = plugin_dir.join("skills").join(skill.dir_name);
        std::fs::create_dir_all(&skill_dir)
            .with_context(|| format!("Failed to create skill dir: {}", skill_dir.display()))?;

        let skill_file = skill_dir.join("SKILL.md");
        std::fs::write(&skill_file, skill.content)
            .with_context(|| format!("Failed to write {}", skill_file.display()))?;
    }

    Ok(true)
}

/// Ensures the folktech-dev-toolkit plugin is installed.
fn ensure_folktech_dev_toolkit(plugins_dir: &Path) -> Result<bool> {
    scaffold_builtin_plugin(
        plugins_dir,
        FOLKTECH_PLUGIN_NAME,
        FOLKTECH_PLUGIN_VERSION,
        PLUGIN_TOML,
        SKILLS,
    )
}

/// Ensures the forge-superpowers plugin is installed.
fn ensure_superpowers_plugin(plugins_dir: &Path) -> Result<bool> {
    scaffold_builtin_plugin(
        plugins_dir,
        SUPERPOWERS_PLUGIN_NAME,
        SUPERPOWERS_PLUGIN_VERSION,
        SUPERPOWERS_PLUGIN_TOML,
        SUPERPOWERS_SKILLS,
    )
}

fn ensure_dev_tools_plugin(plugins_dir: &Path) -> Result<bool> {
    scaffold_builtin_plugin(plugins_dir, DEV_TOOLS_NAME, DEV_TOOLS_VERSION, DEV_TOOLS_TOML, DEV_TOOLS_SKILLS)
}

fn ensure_document_tools_plugin(plugins_dir: &Path) -> Result<bool> {
    scaffold_builtin_plugin(plugins_dir, DOC_TOOLS_NAME, DOC_TOOLS_VERSION, DOC_TOOLS_TOML, DOC_TOOLS_SKILLS)
}

fn ensure_plugin_dev_plugin(plugins_dir: &Path) -> Result<bool> {
    scaffold_builtin_plugin(plugins_dir, PLUGIN_DEV_NAME, PLUGIN_DEV_VERSION, PLUGIN_DEV_TOML, PLUGIN_DEV_SKILLS)
}

/// Ensures all built-in plugins are installed. Called during startup.
/// Only creates plugins that are not already present (idempotent).
/// Returns the number of plugins that were newly created.
pub fn ensure_builtin_plugins(plugins_dir: &Path) -> Result<usize> {
    let mut count = 0;
    if ensure_folktech_dev_toolkit(plugins_dir)? { count += 1; }
    if ensure_superpowers_plugin(plugins_dir)? { count += 1; }
    if ensure_dev_tools_plugin(plugins_dir)? { count += 1; }
    if ensure_document_tools_plugin(plugins_dir)? { count += 1; }
    if ensure_plugin_dev_plugin(plugins_dir)? { count += 1; }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manager::PluginManager;
    use crate::plugins::manifest;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // folktech-dev-toolkit tests (existing)
    // -----------------------------------------------------------------------

    #[test]
    fn test_creates_directory_structure() {
        let tmp = TempDir::new().unwrap();
        let count = ensure_builtin_plugins(tmp.path()).unwrap();
        assert_eq!(count, 5);

        let plugin_dir = tmp.path().join("folktech-dev-toolkit");
        assert!(plugin_dir.is_dir());
        assert!(plugin_dir.join("plugin.toml").is_file());
        assert!(plugin_dir.join(".version").is_file());
        assert!(plugin_dir.join("skills").is_dir());
        assert!(plugin_dir.join("skills/secure-coding").is_dir());
        assert!(plugin_dir.join("skills/tdd-patterns").is_dir());
        assert!(plugin_dir.join("skills/perf-patterns").is_dir());
        assert!(plugin_dir.join("skills/dependency-audit").is_dir());
        assert!(plugin_dir.join("skills/doc-patterns").is_dir());
        assert!(plugin_dir.join("skills/code-quality").is_dir());
    }

    #[test]
    fn test_creates_all_six_skill_files() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let plugin_dir = tmp.path().join("folktech-dev-toolkit");
        let expected_skills = [
            "secure-coding",
            "tdd-patterns",
            "perf-patterns",
            "dependency-audit",
            "doc-patterns",
            "code-quality",
        ];

        for skill_name in &expected_skills {
            let skill_path = plugin_dir.join("skills").join(skill_name).join("SKILL.md");
            assert!(
                skill_path.is_file(),
                "Missing SKILL.md for {skill_name}"
            );
            let content = std::fs::read_to_string(&skill_path).unwrap();
            assert!(
                !content.trim().is_empty(),
                "SKILL.md for {skill_name} is empty"
            );
        }
    }

    #[test]
    fn test_plugin_toml_parses_as_valid_manifest() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let plugin_dir = tmp.path().join("folktech-dev-toolkit");
        let m = manifest::load_manifest(&plugin_dir).unwrap();
        assert_eq!(m.plugin.name, "folktech-dev-toolkit");
        assert_eq!(m.plugin.version, "2.0.0");
        assert_eq!(m.plugin.author, "FolkTech AI");
        assert!(m.tools.is_empty());
        assert!(m.hooks.is_empty());
    }

    #[test]
    fn test_idempotent_second_call_returns_zero() {
        let tmp = TempDir::new().unwrap();
        let first = ensure_builtin_plugins(tmp.path()).unwrap();
        assert_eq!(first, 5);

        // Modify a file to prove second call doesn't overwrite
        let version_path = tmp.path().join("folktech-dev-toolkit/.version");
        std::fs::write(&version_path, "modified").unwrap();

        let second = ensure_builtin_plugins(tmp.path()).unwrap();
        assert_eq!(second, 0);

        // Confirm the file was NOT overwritten
        let content = std::fs::read_to_string(&version_path).unwrap();
        assert_eq!(content, "modified");
    }

    #[test]
    fn test_version_file_written() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let version_path = tmp.path().join("folktech-dev-toolkit/.version");
        let content = std::fs::read_to_string(version_path).unwrap();
        assert_eq!(content, "2.0.0");
    }

    #[test]
    fn test_skills_loadable_via_plugin_manager() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        mgr.load_all().unwrap();

        let skills = mgr.get_skills();
        // 6 from folktech-dev-toolkit + 11 from forge-superpowers = 17
        assert_eq!(skills.len(), 38);

        let triggers: Vec<&str> = skills.iter().map(|s| s.trigger.as_str()).collect();
        assert!(triggers.contains(&"/secure"));
        assert!(triggers.contains(&"/tdd"));
        assert!(triggers.contains(&"/perf"));
        assert!(triggers.contains(&"/audit-dep"));
        assert!(triggers.contains(&"/doc"));
        assert!(triggers.contains(&"/quality"));
    }

    #[test]
    fn test_plugin_toml_has_correct_skill_count() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let plugin_dir = tmp.path().join("folktech-dev-toolkit");
        let m = manifest::load_manifest(&plugin_dir).unwrap();
        assert_eq!(m.skills.len(), 6);
    }

    #[test]
    fn test_path_traversal_blocked_in_plugin_name() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let plugin_dir = tmp.path().join("folktech-dev-toolkit");
        let canonical_plugin = plugin_dir.canonicalize().unwrap();
        let canonical_parent = tmp.path().canonicalize().unwrap();
        assert!(
            canonical_plugin.starts_with(&canonical_parent),
            "Plugin directory escaped plugins_dir"
        );

        for entry in walkdir(tmp.path()) {
            let canonical = entry.canonicalize().unwrap();
            assert!(
                canonical.starts_with(&canonical_parent),
                "File {} escapes plugins_dir",
                entry.display()
            );
        }
    }

    /// Recursively collect all file paths under a directory.
    fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    files.extend(walkdir(&path));
                } else {
                    files.push(path);
                }
            }
        }
        files
    }

    // --- Security red tests (P0: path security) ---

    #[test]
    fn test_plugin_name_constant_has_no_traversal() {
        for name in [FOLKTECH_PLUGIN_NAME, SUPERPOWERS_PLUGIN_NAME] {
            assert!(!name.contains(".."), "{name} contains ..");
            assert!(!name.contains('/'), "{name} contains /");
            assert!(!name.contains('\\'), "{name} contains \\");
            assert!(!name.starts_with('/'), "{name} starts with /");
            assert!(
                name.chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_'),
                "Plugin name {name} contains invalid characters"
            );
        }
    }

    #[test]
    fn test_skill_content_not_empty() {
        for skill in SKILLS {
            assert!(
                !skill.content.trim().is_empty(),
                "Skill {} has empty content",
                skill.dir_name
            );
        }
        for skill in SUPERPOWERS_SKILLS {
            assert!(
                !skill.content.trim().is_empty(),
                "Superpowers skill {} has empty content",
                skill.dir_name
            );
        }
    }

    // -----------------------------------------------------------------------
    // forge-superpowers tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_superpowers_creates_all_eleven_skill_files() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let plugin_dir = tmp.path().join("forge-superpowers");
        let expected_skills = [
            "brainstorming",
            "writing-plans",
            "executing-plans",
            "test-driven-development",
            "systematic-debugging",
            "requesting-code-review",
            "receiving-code-review",
            "dispatching-parallel-agents",
            "subagent-driven-development",
            "verification-before-completion",
            "finishing-a-development-branch",
        ];

        for skill_name in &expected_skills {
            let skill_path = plugin_dir.join("skills").join(skill_name).join("SKILL.md");
            assert!(
                skill_path.is_file(),
                "Missing SKILL.md for superpowers skill {skill_name}"
            );
            let content = std::fs::read_to_string(&skill_path).unwrap();
            assert!(
                !content.trim().is_empty(),
                "SKILL.md for superpowers skill {skill_name} is empty"
            );
        }
    }

    #[test]
    fn test_superpowers_plugin_toml_parses_correctly() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let plugin_dir = tmp.path().join("forge-superpowers");
        let m = manifest::load_manifest(&plugin_dir).unwrap();
        assert_eq!(m.plugin.name, "forge-superpowers");
        assert_eq!(m.plugin.version, "1.0.0");
        assert_eq!(m.plugin.author, "FolkTech AI");
        assert_eq!(m.skills.len(), 11);
        assert!(m.tools.is_empty());
        assert!(m.hooks.is_empty());
    }

    #[test]
    fn test_superpowers_idempotent() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        // Modify version file to prove no overwrite
        let version_path = tmp.path().join("forge-superpowers/.version");
        std::fs::write(&version_path, "modified").unwrap();

        let second = ensure_superpowers_plugin(tmp.path()).unwrap();
        assert!(!second, "Second call should return false (no-op)");

        let content = std::fs::read_to_string(&version_path).unwrap();
        assert_eq!(content, "modified");
    }

    #[test]
    fn test_superpowers_version_file_written() {
        let tmp = TempDir::new().unwrap();
        ensure_builtin_plugins(tmp.path()).unwrap();

        let version_path = tmp.path().join("forge-superpowers/.version");
        let content = std::fs::read_to_string(version_path).unwrap();
        assert_eq!(content, "1.0.0");
    }

    #[test]
    fn test_superpowers_skills_loadable_via_plugin_manager() {
        let tmp = TempDir::new().unwrap();
        ensure_superpowers_plugin(tmp.path()).unwrap();

        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        mgr.load_all().unwrap();

        let skills = mgr.get_skills();
        assert_eq!(skills.len(), 11);

        let triggers: Vec<&str> = skills.iter().map(|s| s.trigger.as_str()).collect();
        assert!(triggers.contains(&"/brainstorm"));
        assert!(triggers.contains(&"/plan"));
        assert!(triggers.contains(&"/execute"));
        assert!(triggers.contains(&"/tdd-workflow"));
        assert!(triggers.contains(&"/debug"));
        assert!(triggers.contains(&"/review"));
        assert!(triggers.contains(&"/review-feedback"));
        assert!(triggers.contains(&"/parallel"));
        assert!(triggers.contains(&"/subagent-dev"));
        assert!(triggers.contains(&"/verify"));
        assert!(triggers.contains(&"/finish"));
    }

    #[test]
    fn test_superpowers_directory_stays_within_plugins_dir() {
        let tmp = TempDir::new().unwrap();
        ensure_superpowers_plugin(tmp.path()).unwrap();

        let plugin_dir = tmp.path().join("forge-superpowers");
        let canonical_plugin = plugin_dir.canonicalize().unwrap();
        let canonical_parent = tmp.path().canonicalize().unwrap();
        assert!(
            canonical_plugin.starts_with(&canonical_parent),
            "Superpowers plugin directory escaped plugins_dir"
        );

        for entry in walkdir(&plugin_dir) {
            let canonical = entry.canonicalize().unwrap();
            assert!(
                canonical.starts_with(&canonical_parent),
                "File {} escapes plugins_dir",
                entry.display()
            );
        }
    }

    #[test]
    fn test_superpowers_skill_triggers_unique() {
        let tmp = TempDir::new().unwrap();
        ensure_superpowers_plugin(tmp.path()).unwrap();

        let plugin_dir = tmp.path().join("forge-superpowers");
        let m = manifest::load_manifest(&plugin_dir).unwrap();

        let mut triggers = std::collections::HashSet::new();
        for skill in &m.skills {
            if let Some(ref t) = skill.trigger {
                assert!(
                    triggers.insert(t.clone()),
                    "Duplicate trigger: {t}"
                );
            }
        }
    }
}
