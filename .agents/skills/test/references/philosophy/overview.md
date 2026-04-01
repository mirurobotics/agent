
## Philosophy

Testing is a cornerstone of software at Miru. Everything is tested. Everything.

Whatever can be tested should be tested:

- Source code (obviously)
- External clients (sanity checks only)
- Database migrations
- Shell scripts
- etc.

Testing provides a litany of benefits:

- Improves software design and code quality
- Improves product reliability
- Increases confidence that features and refactors don't break existing functionality

## Test types and coverage

Short overview of each testing topic. **Language-specific checklists** (Go, JavaScript/TypeScript, Python, Rust) live in `testing/checklists/` and contain coverage, quality, and organization checklist items.

**Unit tests** — Small, isolated tests with no external dependencies; everything mocked, runs very fast. Full explanation: [`unit-tests.mdc`](unit-tests.mdc).

**Integration tests** — Multiple components together; mock as little as possible but keep runs fast (e.g. mock S3). Full explanation: [`integration-tests.mdc`](integration-tests.mdc).

**End-to-end tests** — Full user or system flows; not yet implemented. Full explanation: [`e2e-tests.mdc`](e2e-tests.mdc).

**Test coverage** — Aim for every testable line; 80%+ required; meaningful tests and edge cases matter more than coverage percentage. Full explanation: [`test-coverage.mdc`](test-coverage.mdc).
