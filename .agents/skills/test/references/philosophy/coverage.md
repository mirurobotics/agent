
# Test Coverage

We want every line of code that is reasonable and useful to test to be tested. Test coverage isn't a vanity metric—we use it as a tool to help check that our code is well-tested. However, coverage is no substitute for well thought through tests and edge cases.

High coverage numbers alone don't guarantee quality tests. A test suite with 90% coverage that only tests happy paths is less valuable than a test suite with 70% coverage that thoroughly tests edge cases, error conditions, and boundary values. Use coverage as a guide to identify untested code paths, but always prioritize writing meaningful tests that validate behavior, not just execution.

At Miru, we require **80%+ test coverage** for every package/module. No exceptions.

For language-specific coverage tooling and conventions, see the testing checklists (e.g. `testing/checklists/go.mdc`).
