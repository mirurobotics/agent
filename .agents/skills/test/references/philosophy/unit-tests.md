
# Unit Tests

Unit tests test small, primitive functions with no external dependencies. Unit tests are completely mocked.

External dependencies are typically clients: database, S3, SAAS providers, etc. The file system is also considered an external dependency. If a test requires use of the file system, we do not consider it a unit test.

Unit tests should run *very* fast. If unit tests are running slow, something is very wrong.

For language-specific placement, naming, and patterns, see the testing checklists (e.g. `testing/checklists/go.mdc`).
