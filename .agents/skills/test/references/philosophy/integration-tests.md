
# Integration Tests

Integration tests focus on multiple components working together. Mocking is still heavily used, but some clients are not mocked. Our philosophy is to mock as little as possible. However, we need integration tests to run relatively quickly still so most dependencies (such as S3) should still be mocked for practicality.

For language-specific placement and patterns, see the testing checklists (e.g. `testing/checklists/go.mdc`).
