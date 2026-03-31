
# ARCHITECTURE.md for a repo

Repositories (especially in the 10k–200k line range) benefit from a single **architecture document** at the repo root (e.g. `ARCHITECTURE.md` or `docs/ARCHITECTURE.md`). It gives new contributors a map of the codebase and explains where to change code for a given concern.

**Keep it short and stable.** Only document what changes rarely. Don’t try to keep it in sync with every refactor—revisit it periodically (e.g. a few times a year).

**Suggested contents:**
1. **Problem and scope** — What this repo is for and what’s in scope.
2. **Codemap** — High-level modules and how they relate (“where is the thing that does X?” and “what does the thing I’m looking at do?”). Name important files, modules, or types; avoid deep implementation detail. Prefer telling readers to use symbol search over maintaining many links.
3. **Invariants** — Important rules the architecture relies on, especially ones that are expressed by *absence* (e.g. “nothing in the model layer depends on the views”).
4. **Boundaries** — Where layers or systems meet; what is allowed to cross those boundaries.
5. **Cross-cutting concerns** — Logging, configuration, error handling, etc., and where they live.

This applies to any repo (backend, frontend, etc.). For a working example, see the root `ARCHITECTURE.md` in the workspace.
