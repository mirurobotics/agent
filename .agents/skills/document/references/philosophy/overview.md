
# Documentation standards

The best documentation is good code. Even so, good documentation is still necessary. Our philosophy: **write only the documentation that is necessary and no more.** Anything that just repeats what the code says is a DRY violation and should be avoided.

The checklist items for each topic live in the **language-specific** documentation rules. Use the checklist for the language you’re working in.

---

## Docstrings

When and how to write docstrings; when to omit them; code examples in docs.

**Full explanation:** [`docstrings.mdc`](docstrings.mdc).  
**Checklist:** `documentation/checklists/` → Docstrings (or JSDoc / Doc comments) section for your language (Go, JavaScript/TypeScript, Python, Rust).

---

## Inline comments

When to use inline comments and what they should explain (the “why,” not the “what”).

**Full explanation:** [`comments.mdc`](comments.mdc).  
**Checklist:** `documentation/checklists/` → Inline comments / Inline documentation section for your language.

---

## ARCHITECTURE.md for a repo

How to add and maintain an architecture document at the root of a repository (codemap, invariants, boundaries).

**Full explanation:** [`architecture.mdc`](architecture.mdc).  
No per-language checklist; apply when adding or updating an `ARCHITECTURE.md` (or equivalent) in a repo.

---

**Language checklists:** `documentation/checklists/go.mdc`, `documentation/checklists/javascript.mdc`, `documentation/checklists/python.mdc`, `documentation/checklists/rust.mdc`. Each contains the full documentation checklist for that language (docstrings, inline comments, examples, and language-specific conventions).
