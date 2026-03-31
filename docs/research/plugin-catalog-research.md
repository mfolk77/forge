# Forge Plugin Catalog Research

**Date:** 2026-03-29
**Purpose:** Curated list of high-quality plugins, skills, and tools for Forge's built-in plugin catalog.
**Sources:** Claude Code ecosystem, HuggingFace, GitHub, MCP server ecosystem.

---

## Selection Criteria

- Real adoption (stars, installs, or proven production use)
- Clear utility for developer workflows
- Feasible to implement as Forge plugin catalog entries
- Bias toward tools that complement local/offline-first AI coding

---

## Catalog Entries

### 1. Superpowers Suite

```
Name: superpowers
Category: FolkTechCore
Description: Multi-skill orchestration suite — brainstorming, plan writing, plan execution, TDD, systematic debugging, code review, parallel agent dispatch, and git worktree management.
Source: Claude Code plugin "superpowers" (hesreallyhim/awesome-claude-code ecosystem, 34k+ stars on the awesome list)
Why: Most comprehensive single plugin in the Claude Code ecosystem. Covers the entire dev lifecycle from ideation to merge. Each skill is independently invokable.
Priority: P1 (must have)
```

### 2. All-Skills Collection (Curated Subset)

```
Name: document-toolkit
Category: Utility
Description: Create and manipulate DOCX, XLSX, PPTX, and PDF files programmatically from the terminal.
Source: Claude Code plugin "all-skills" (docx, xlsx, pptx, pdf skills)
Why: Extremely popular in the Claude Code ecosystem. Fills a gap where CLI tools typically can't produce Office-format documents.
Priority: P2 (should have)
```

```
Name: changelog-generator
Category: Workflow
Description: Automatically generate user-facing changelogs from git commit history, categorizing and humanizing technical commits.
Source: Claude Code plugin "all-skills" (changelog-generator skill)
Why: Saves hours of manual changelog work. Direct git integration makes it ideal for Forge's local-first model.
Priority: P2 (should have)
```

```
Name: content-research-writer
Category: Utility
Description: Research-backed content writing with citations, outline iteration, and section-by-section feedback.
Source: Claude Code plugin "all-skills" (content-research-writer skill)
Why: Proven workflow for technical writing, blog posts, and documentation drafts.
Priority: P3 (nice to have)
```

### 3. All-Agents (Role-Based Agent Personas)

```
Name: agent-personas
Category: AI
Description: Specialized agent personas — backend architect, frontend developer, iOS developer, code reviewer, security auditor — each with domain-tuned system prompts and tool access.
Source: Claude Code plugin "all-agents" (alirezarezvani/claude-skills, 8.2k stars)
Why: Role-based personas dramatically improve output quality for domain-specific tasks. 192+ skills across engineering, product, and compliance.
Priority: P2 (should have)
```

### 4. Commit Commands

```
Name: commit-commands
Category: Workflow
Description: Intelligent git commit, push, and PR creation with automatic message generation from staged diffs.
Source: Claude Code plugin "commit-commands" (commit, commit-push-pr)
Why: One of the most-used Claude Code plugins. Streamlines the most repetitive part of development.
Priority: P1 (must have)
```

### 5. Code Review

```
Name: code-review
Category: Workflow
Description: AI-powered pull request review with structured feedback on correctness, security, performance, and style.
Source: Claude Code plugin "code-review"
Why: Core developer workflow. Reduces review latency and catches issues before human reviewers.
Priority: P1 (must have)
```

### 6. PR Review Toolkit

```
Name: pr-review-toolkit
Category: Workflow
Description: Multi-agent PR analysis — test analyzer, type design reviewer, code simplifier, and silent-failure-hunter that detects swallowed errors.
Source: Claude Code plugin "pr-review-toolkit"
Why: Goes beyond basic code review with specialized sub-agents. The silent-failure-hunter alone justifies inclusion.
Priority: P2 (should have)
```

### 7. Feature Dev

```
Name: feature-dev
Category: Workflow
Description: Guided feature development workflow — codebase analysis, architecture planning, implementation with review checkpoints.
Source: Claude Code plugin "feature-dev"
Why: Structured approach prevents the "just start coding" anti-pattern. Forces architecture-first thinking.
Priority: P2 (should have)
```

### 8. Hookify

```
Name: hookify
Category: DevTools
Description: Automatically create git hooks and automation rules from conversation analysis or explicit instructions.
Source: Claude Code plugin "hookify"
Why: Bridges the gap between "I want X to happen automatically" and actually configuring it. Unique capability.
Priority: P2 (should have)
```

### 9. Plugin Dev Kit

```
Name: plugin-dev
Category: FolkTechCore
Description: End-to-end plugin creation — scaffolding, hook development, skill authoring, command design, MCP integration, and settings management.
Source: Claude Code plugin "plugin-dev" (create-plugin, plugin-structure, hook-development, skill-development, etc.)
Why: Essential for ecosystem growth. Makes it easy for users to extend Forge with their own plugins.
Priority: P1 (must have)
```

### 10. CLAUDE.md Management (adapted as FTAI.md Management)

```
Name: config-auditor
Category: FolkTechCore
Description: Audit and improve project configuration files (FTAI.md / CLAUDE.md). Scans for quality issues, suggests improvements, updates with session learnings.
Source: Claude Code plugin "claude-md-management"
Why: Configuration quality directly impacts output quality. Auto-improvement loop is high value.
Priority: P2 (should have)
```

### 11. Context7 — Library Documentation Fetcher

```
Name: context7
Category: Integration
Description: Fetch up-to-date documentation for any library, framework, or SDK directly into context. Prevents hallucination from stale training data.
Source: upstash/context7 (51k+ GitHub stars), Claude Code plugin "context7"
Why: One of the highest-starred tools in the entire MCP ecosystem. Solves a fundamental problem with LLM-assisted coding.
Priority: P1 (must have)
```

### 12. Frontend Design

```
Name: frontend-design
Category: Utility
Description: Generate production-grade frontend interfaces with high design quality. Avoids generic AI aesthetics with creative, polished output.
Source: Claude Code plugin "frontend-design"
Why: Addresses the common complaint that AI-generated UIs look generic. Opinionated design system approach.
Priority: P3 (nice to have)
```

### 13. Skill Creator

```
Name: skill-creator
Category: FolkTechCore
Description: Create, modify, and benchmark custom skills with eval-driven development and variance analysis.
Source: Claude Code plugin "skill-creator"
Why: Meta-tool that accelerates ecosystem development. Eval integration ensures skills actually work.
Priority: P2 (should have)
```

---

### MCP Server Integrations

### 14. GitHub MCP Server

```
Name: mcp-github
Category: Integration
Description: Full GitHub API access — issues, PRs, repos, actions, code search — via Model Context Protocol.
Source: github/github-mcp-server (28.4k stars)
Why: Official GitHub MCP server. Most starred MCP integration. Essential for any coding workflow.
Priority: P1 (must have)
```

### 15. Playwright MCP (Browser Automation)

```
Name: mcp-playwright
Category: Integration
Description: Browser automation and testing via Playwright through MCP. Navigate, screenshot, interact with web pages.
Source: microsoft/playwright-mcp (30k stars)
Why: Most starred MCP server overall. Official Microsoft project. Enables web testing and scraping workflows.
Priority: P1 (must have)
```

### 16. Figma MCP

```
Name: mcp-figma
Category: Integration
Description: Read Figma designs, extract layout context, and generate code from design files via MCP.
Source: GLips/Figma-Context-MCP (14k stars)
Why: Bridges design-to-code gap. High adoption among frontend developers.
Priority: P2 (should have)
```

### 17. AWS MCP Servers

```
Name: mcp-aws
Category: Integration
Description: Official AWS MCP servers for S3, Lambda, DynamoDB, CloudFormation, and other AWS services.
Source: awslabs/mcp (8.6k stars)
Why: Official AWS project. Essential for cloud-native development workflows.
Priority: P2 (should have)
```

### 18. Firecrawl MCP (Web Scraping)

```
Name: mcp-firecrawl
Category: Integration
Description: Web scraping, crawling, and search via Firecrawl's MCP server. Converts web pages to clean markdown.
Source: firecrawl/firecrawl-mcp-server (5.9k stars)
Why: Clean web-to-markdown conversion is critical for feeding web content into local models.
Priority: P2 (should have)
```

### 19. Chrome DevTools MCP

```
Name: mcp-chrome-devtools
Category: Integration
Description: Chrome DevTools protocol access for AI agents — inspect DOM, network, console, performance.
Source: ChromeDevTools/chrome-devtools-mcp (32.4k stars)
Why: Official Chrome project. Enables real browser debugging from the terminal.
Priority: P2 (should have)
```

### 20. Database Toolbox MCP

```
Name: mcp-database
Category: Integration
Description: MCP server for database operations — query, schema inspection, and migration across Postgres, MySQL, SQLite, and more.
Source: googleapis/genai-toolbox (13.6k stars)
Why: Official Google project. Database interaction is a core developer need.
Priority: P2 (should have)
```

### 21. Serena (Semantic Code Intelligence)

```
Name: mcp-serena
Category: Integration
Description: Semantic code retrieval and editing — AST-aware search, symbol navigation, and intelligent refactoring via MCP.
Source: oraios/serena (22.3k stars)
Why: Goes beyond text search with actual code understanding. Very high star count for a coding tool.
Priority: P1 (must have)
```

### 22. MCP Inspector

```
Name: mcp-inspector
Category: DevTools
Description: Visual testing and debugging tool for MCP servers. Inspect requests, responses, and protocol conformance.
Source: modelcontextprotocol/inspector (9.3k stars)
Why: Official MCP project. Essential for anyone building or debugging MCP integrations.
Priority: P2 (should have)
```

### 23. Atlassian MCP (Jira + Confluence)

```
Name: mcp-atlassian
Category: Integration
Description: Jira and Confluence access via MCP — read/write issues, search documentation, manage sprints.
Source: sooperset/mcp-atlassian (4.8k stars)
Why: Jira is ubiquitous in enterprise development. Bridges project management and coding.
Priority: P3 (nice to have)
```

---

### Developer CLI Tools (Wrappable as Plugins)

### 24. Probe (Semantic Code Search)

```
Name: probe-search
Category: DevTools
Description: AI-friendly semantic code search combining ripgrep speed with tree-sitter AST parsing for context-aware results.
Source: probelabs/probe (519 stars, but uniquely useful for AI coding agents)
Why: Purpose-built for feeding code context to LLMs. Combines speed (ripgrep) with understanding (tree-sitter). Directly useful for Forge's search module.
Priority: P2 (should have)
```

### 25. CodeGraph Context

```
Name: codegraph
Category: DevTools
Description: Index local code into a graph database for rich cross-reference context — call graphs, dependency maps, symbol resolution.
Source: CodeGraphContext/CodeGraphContext (2.7k stars)
Why: Graph-based code understanding is significantly better than flat text search for large codebases.
Priority: P2 (should have)
```

### 26. Claude Memory (Session Persistence)

```
Name: session-memory
Category: FolkTechCore
Description: Automatically capture, compress, and reinject session context across coding sessions. Persistent memory for AI coding agents.
Source: thedotmack/claude-mem (43.7k stars — highest-starred Claude Code plugin)
Why: Most starred plugin in the entire Claude Code ecosystem. Solves the fundamental context loss problem between sessions.
Priority: P1 (must have)
```

### 27. Claude HUD (Session Observability)

```
Name: session-hud
Category: DevTools
Description: Real-time observability dashboard — context usage, active tools, running agents, todo progress, and token consumption.
Source: jarrodwatts/claude-hud (15.5k stars)
Why: Second most starred Claude Code plugin. Critical for understanding and optimizing agent behavior.
Priority: P1 (must have)
```

### 28. Compound Engineering

```
Name: compound-engineering
Category: Workflow
Description: Office-grade compound engineering patterns — multi-agent orchestration, structured planning, and execution for complex engineering tasks.
Source: EveryInc/compound-engineering-plugin (12.1k stars)
Why: High adoption. Brings structured engineering methodology to AI-assisted development.
Priority: P2 (should have)
```

### 29. Ruflo (Agent Orchestration)

```
Name: agent-orchestrator
Category: AI
Description: Multi-agent swarm orchestration — deploy coordinated agent teams with vector-based memory, systematic planning, and security guardrails.
Source: ruvnet/ruflo (28.7k stars)
Why: Leading agent orchestration platform. Enterprise-grade multi-agent patterns directly applicable to Forge's architecture.
Priority: P2 (should have)
```

### 30. FastMCP (MCP Server Framework)

```
Name: fastmcp-framework
Category: DevTools
Description: Fast, Pythonic framework for building MCP servers and clients. Simplifies creating custom tool integrations.
Source: PrefectHQ/fastmcp (24.2k stars)
Why: De facto standard for building MCP servers in Python. Essential for users who want to create custom Forge integrations.
Priority: P2 (should have)
```

### 31. Git MCP (Repo Documentation)

```
Name: mcp-git-docs
Category: Integration
Description: Turn any GitHub repository into an MCP-accessible documentation source. Eliminates code hallucinations by grounding in actual repo content.
Source: idosal/git-mcp (7.8k stars)
Why: Solves hallucination for open-source library usage. Complementary to context7.
Priority: P2 (should have)
```

### 32. Webapp Testing (Playwright-Based)

```
Name: webapp-testing
Category: DevTools
Description: Interactive web app testing toolkit — verify frontend functionality, capture screenshots, read browser logs, debug UI behavior.
Source: Claude Code plugin "all-skills" (webapp-testing skill)
Why: Fills the gap between writing frontend code and verifying it actually works.
Priority: P3 (nice to have)
```

### 33. Security Audit

```
Name: security-audit
Category: FolkTechCore
Description: Automated security audit — SAST analysis, dependency scanning, secrets detection, and vulnerability reporting.
Source: Claude Code built-in audit skill + arm/metis (495 stars, AI-driven deep security review)
Why: Aligns with FolkTech Secure Coding Standard requirement. Every codebase needs automated security scanning.
Priority: P1 (must have)
```

### 34. Agnix (Agent Config Linter)

```
Name: agent-linter
Category: DevTools
Description: Lint and validate agent configuration files — CLAUDE.md, AGENTS.md, SKILL.md, hooks, MCP configs — with autofixes.
Source: agent-sh/agnix (128 stars but filling a unique niche)
Why: No other tool validates agent configuration files. Prevents silent misconfigurations.
Priority: P3 (nice to have)
```

### 35. n8n MCP (Workflow Automation)

```
Name: mcp-n8n
Category: Integration
Description: Build and manage n8n automation workflows from the terminal via MCP.
Source: czlonkowski/n8n-mcp (17.1k stars)
Why: n8n is the leading open-source workflow automation platform (181k+ stars). MCP bridge enables AI-driven workflow creation.
Priority: P3 (nice to have)
```

### 36. HuggingFace Hub Integration

```
Name: mcp-huggingface
Category: Integration
Description: Search models, datasets, and papers on HuggingFace Hub. Fetch documentation, run inference on Spaces, and discover trending research.
Source: HuggingFace official MCP server (available in Claude Code MCP ecosystem)
Why: Official HuggingFace integration. Essential for ML/AI development workflows and model discovery.
Priority: P2 (should have)
```

### 37. Accessibility Agents

```
Name: accessibility-review
Category: Workflow
Description: Eleven specialized accessibility review agents enforcing WCAG 2.2 AA compliance across AI-generated code.
Source: Community-Access/accessibility-agents (206 stars)
Why: Unique focus area. AI tools frequently generate inaccessible code. Prevents shipping a11y violations.
Priority: P3 (nice to have)
```

---

## Summary by Priority

| Priority | Count | Entries |
|----------|-------|---------|
| **P1 (must have)** | 10 | superpowers, commit-commands, code-review, plugin-dev, context7, mcp-github, mcp-playwright, mcp-serena, session-memory, session-hud, security-audit |
| **P2 (should have)** | 17 | document-toolkit, changelog-generator, agent-personas, pr-review-toolkit, feature-dev, hookify, config-auditor, skill-creator, mcp-figma, mcp-aws, mcp-firecrawl, mcp-chrome-devtools, mcp-database, mcp-inspector, probe-search, codegraph, compound-engineering, agent-orchestrator, fastmcp-framework, mcp-git-docs, mcp-huggingface |
| **P3 (nice to have)** | 7 | content-research-writer, frontend-design, mcp-atlassian, webapp-testing, agent-linter, mcp-n8n, accessibility-review |

## Implementation Notes

1. **MCP-first approach**: Most integrations (GitHub, Playwright, Figma, AWS, databases) should be implemented as MCP server wrappers in Forge's plugin system. This aligns with the industry standard and allows reuse of existing MCP server implementations.

2. **Skill-based plugins** (superpowers, code-review, feature-dev) are best implemented as prompt-based skills with structured workflows — no external dependencies needed.

3. **Session memory** (claude-mem pattern) should be considered for core Forge integration rather than just a plugin, given its 43k+ star adoption and fundamental importance to multi-session workflows.

4. **Context7** deserves special attention — at 51k stars it is the most adopted tool in the broader MCP ecosystem and solves a problem every Forge user will hit (stale library knowledge in local models).

5. **Security audit** is non-negotiable per FolkTech Secure Coding Standard and should ship as a built-in, not an optional plugin.
