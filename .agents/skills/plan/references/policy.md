# Execution Plans (ExecPlans)

This document describes the requirements for an **ExecPlan**: a design document that a Cursor agent (or human) can follow to deliver a working feature or system change. Treat the reader as a complete beginner to this repository: they have only the current working tree and the single ExecPlan file. There is no memory of prior plans and no external context.

**Reference:** [OpenAI cookbook: Using PLANS.md for multi-hour problem solving](https://developers.openai.com/cookbook/articles/codex_exec_plans/).

---

## How to use ExecPlans and this file

**When authoring an ExecPlan:** Follow this document to the letter. Start from the skeleton below and flesh it out as you research. Be thorough; the plan must be self-contained.

**When implementing an ExecPlan:** Do not prompt the user for "next steps"; proceed to the next milestone. Keep all sections up to date—add or split entries in Progress at every stopping point. Resolve ambiguities autonomously and commit frequently. When work spans submodules, run git commands from each affected repo root (e.g. `backend/`, `frontend/`); name repo roots and paths explicitly in the plan.

**When revising an ExecPlan:** Record decisions in the Decision Log so it is clear why any change was made. ExecPlans are living documents; it should always be possible to restart from only the ExecPlan.

### Plan placement

An ExecPlan lives inside the repository where code will be written. If a plan spans multiple repositories, please create a separate plan for each repository. Each repository's plan should focus on its own implementation and should not be identical to other repository's plan. Duplicate overview information is suggested so that all plans are self-contained.

Reading from other repos during research and authoring is encouraged, but the plan file itself must live in the repo that owns the work.

### Plan lifecycle and directories

Use these directories (relative to the chosen repo root) to make execution status explicit:

- `.agents/exec-plans/backlog/` for draft plans that are still being explored or awaiting approval to implement.
- `.agents/exec-plans/active/` for plans that are approved and currently being implemented.
- `.agents/exec-plans/completed/` for finished plans, with Outcomes & Retrospective filled in.

Promotion flow:

1. Draft in `backlog/`.
2. Move to `active/` only when implementation is approved.
3. Move to `completed/` when implementation is done.

### File naming convention

All new ExecPlan filenames must use:

- `YYYYMMDD-<title>.md`

Where:

- `YYYYMMDD` is the plan creation date in UTC (for example `20260214`).
- `<title>` is a short, lowercase, hyphenated description (for example `agent-ci-e2e-testing`).

Examples:

- `20260214-agent-ci-e2e-testing.md`
- `20260214-restructure-agent-error-handling.md`

Do not rename existing in-use plans only to satisfy this convention; apply it to new plans and opportunistically when plans are naturally migrated.

---

## Non-negotiable requirements

- **Self-contained.** The ExecPlan in its current form contains all knowledge and instructions needed for a novice to succeed. No "as defined in the architecture doc" or external links for essential context; embed what is needed.
- **Living document.** Revise as progress is made, discoveries occur, and design decisions are finalized. Each revision remains self-contained.
- **Novice-guiding.** A reader with no prior knowledge of this repo can implement the feature end-to-end from the plan alone.
- **Outcome-focused.** The plan must produce demonstrably working behavior (what the user can do and observe), not merely code that "meets a definition."
- **Plain language.** Define every term of art when first used and say where it appears in this repo (files, commands). Do not rely on undefined jargon.

**Miru-specific:** This is a root repo with submodules (backend, frontend, agent, docs, etc.). When the plan touches multiple repos, name paths relative to the repo that owns them (e.g. `backend/internal/foo/bar.go`, `frontend/app/page.tsx`). State the working directory for every command (e.g. "From `backend/`: run `go test ./...`"). Commit from each repo root separately; do not run `git commit` from the root for submodule changes.

---

## Formatting

- Use standard Markdown: `#` and `##` headings, two newlines after headings, correct list syntax.
- Write in plain prose; prefer sentences over long bullet lists except in Progress (where checklists are mandatory).
- When you need to show commands, diffs, or code inside the plan, use indented blocks rather than nested fenced code blocks.
- When the ExecPlan is the only content of a file (e.g. in `.agents/exec-plans/active/`), do not wrap the whole thing in triple backticks; the file is the plan.

---

## Guidelines

- **Observable outcomes.** State what the user can do after implementation, which commands to run, and what output to expect. Acceptance = behavior a human can verify (e.g. "GET /health returns 200 with body OK"), not internal attributes ("added HealthCheck struct").
- **Explicit repo context.** Name files with full repo-relative paths; name functions and modules precisely; say where new files go. If touching multiple submodules, add a short orientation so a novice knows how the parts fit. For every command, give the working directory and exact command line.
- **Idempotent and safe.** Steps should be runnable multiple times without damage or drift. If a step can fail halfway, say how to retry. For destructive or migration steps, spell out backups or fallbacks.
- **Validation required.** Include how to run tests, start the system if relevant, and observe useful behavior. Give expected outputs and error messages. State the exact test commands for the project's toolchain and how to interpret results.

---

## Mandatory sections

Every ExecPlan must contain and maintain these sections. They are not optional.

| Section | Purpose |
|--------|--------|
| **Scope** | Which repositories are read and which are written. List each affected repo and whether it is read-only (research/context) or read-write (code changes). This determines where the plan file lives. |
| **Purpose / Big Picture** | In a few sentences: what someone gains after this change and how they can see it working. User-visible behavior. |
| **Progress** | Checklist of granular steps. Update at every stopping point; use timestamps if helpful. Split partially done work into "done" vs "remaining." |
| **Surprises & Discoveries** | Unexpected behavior, bugs, or insights during implementation. Short evidence (e.g. test output). |
| **Decision Log** | Every non-trivial decision: what was decided, why, date/author. |
| **Outcomes & Retrospective** | At completion (or major milestones): what was achieved, what remains, lessons learned. |
| **Context and Orientation** | Current state relevant to this task as if the reader knows nothing. Key files and modules by full path; definitions of non-obvious terms. |
| **Plan of Work** | Prose sequence of edits and additions. For each edit: file, location (function/module), what to insert or change. Concrete and minimal. |
| **Concrete Steps** | Exact commands to run and where (working directory). When a command produces output, show a short expected transcript. Update as work proceeds. |
| **Validation and Acceptance** | How to start or exercise the system and what to observe. Acceptance phrased as behavior with specific inputs and outputs. If tests: "run &lt;command&gt; and expect &lt;N&gt; passed; test &lt;name&gt; fails before and passes after." |
| **Idempotence and Recovery** | Which steps can be repeated safely; for risky steps, retry or rollback path. |

Optional but useful: **Interfaces and Dependencies** (libraries, types, function signatures that must exist); **Artifacts and Notes** (key transcripts, diffs, snippets as indented examples).

---

## Skeleton of a good ExecPlan

Copy this skeleton into a new file in `.agents/exec-plans/backlog/` named `YYYYMMDD-<title>.md` and fill it in. Move it to `.agents/exec-plans/active/` when implementation is approved.

```markdown
# <Short, action-oriented description>

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `<repo>/` | read-write | <what changes are made here> |
| `<repo>/` | read-only | <why this repo is referenced> |

This plan lives in `<repo>/.agents/exec-plans/` because <reason>.

## Purpose / Big Picture

Explain in a few sentences what someone gains after this change and how they can see it working. State the user-visible behavior you will enable.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Example incomplete step.
- [ ] Example incomplete step.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

(Add entries as you go.)

- Decision: …
  Rationale: …
  Date/Author: …

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Describe the current state relevant to this task as if the reader knows nothing. Name key files and modules by full path (e.g. backend/internal/foo/bar.go). Define any non-obvious terms.

## Plan of Work

Describe in prose the sequence of edits and additions. For each edit: file, location (function/module), what to insert or change. Keep it concrete and minimal.

## Concrete Steps

State the exact commands to run and where (working directory). When a command generates output, show a short expected transcript. Update this section as work proceeds.

## Validation and Acceptance

How to start or exercise the system and what to observe. Phrase acceptance as behavior with specific inputs and outputs. If tests: "run <project test command> and expect <N> passed; the new test <name> fails before the change and passes after."

## Idempotence and Recovery

Which steps can be repeated safely. For risky steps, provide retry or rollback path.
```

---

When you revise a plan, ensure changes are reflected across all sections (including the living-document sections) and add a note at the bottom describing the change and the reason. ExecPlans must describe not just the what but the why.
