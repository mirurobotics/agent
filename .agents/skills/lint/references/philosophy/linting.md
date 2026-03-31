
# Linting

Use linting tools to enforce code style. Follow idiomatic conventions for the language. Enforce a maximum line length (typically 88–100 characters). Format code with the language-specific formatter (e.g. `gofmt`, `prettier`, `black`, `rustfmt`).

**Checklist:** **checklists** → Formatting / Code Style (per language).

## Building custom linters

When off-the-shelf linters aren’t enough, custom linters can enforce project-specific invariants: architecture (e.g. dependency direction, layering), “taste” rules (structured logging, naming for schemas and types, file size limits), and domain requirements (platform-specific reliability, docs structure). Enforce invariants mechanically rather than relying on documentation alone; when a rule keeps getting violated, promote it into code.

**Error messages matter.** With custom lints you control the message—use it to inject clear remediation instructions so both humans and agents can fix issues without extra context. Well-written messages multiply the value of the rule.

**Good candidates for custom linters:** structural checks (e.g. “this layer may not depend on that layer”), consistency checks (e.g. knowledge base is cross-linked and up to date), and static enforcement of patterns that are hard to describe in prose (e.g. “all config is parsed at the boundary”). Pair with structural tests where appropriate. Prefer encoding once and applying everywhere over repeating the rule in docs or review.
