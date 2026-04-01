
# Python-Specific Testing Standards

This file contains Python-specific testing recommendations.

## Test Organization

Python tests are typically organized as:
- **Unit tests**: Placed in `tests/` directory or alongside code as `*_test.py` files
- **Integration tests**: Placed in `tests/integration/` or `tests/` directory
- **Test files**: Named `test_*.py` or `*_test.py` (pytest convention)

## Test Framework

- [ ] `pytest` is the preferred testing framework (over `unittest`)
- [ ] `pytest.fixture` is used for test setup and teardown
- [ ] `pytest.mark.parametrize` is used for parameterized tests
- [ ] `pytest.raises()` is used to test exceptions

## Parameterized Tests

Python uses `pytest.mark.parametrize` for parameterized tests (similar to table-driven tests in Go):

```python
import pytest

@pytest.mark.parametrize("input,expected", [
    ("hello", "HELLO"),
    ("world", "WORLD"),
    ("", ""),
])
def test_uppercase(input: str, expected: str) -> None:
    assert input.upper() == expected
```

Or using a list of dictionaries for more complex cases:

```python
@pytest.mark.parametrize("test_case", [
    {"name": "happy path", "input": Input(...), "expected": Expected(...)},
    {"name": "error case", "input": Input(...), "expect_error": True},
])
def test_function_name(test_case: dict) -> None:
    if test_case.get("expect_error"):
        with pytest.raises(ExpectedError):
            function_name(test_case["input"])
    else:
        result = function_name(test_case["input"])
        assert result == test_case["expected"]
```

## Test Naming

- [ ] Test functions are named `test_<function_name>` or `test_<function_name>_<scenario>`
- [ ] Test classes are named `Test<ClassName>`
- [ ] Test files are named `test_*.py` or `*_test.py`

## Checklist

### Testability
- [ ] dependencies are injectable (interfaces, function parameters, dependency injection)
- [ ] no global state that complicates testing
- [ ] code is structured to be easily testable

### Test Coverage
- [ ] Test the happy path (normal successful operation)
- [ ] Test all error conditions and error messages
- [ ] Test edge cases (nil/null, empty, zero, boundary values)
- [ ] Test any documented behavior or constraints
- [ ] 80+% coverage of the code

### Test Quality
- [ ] Tests are independent and can run in any order
- [ ] Tests clean up after themselves (no leftover files, state)
- [ ] Unit tests don't rely on external services; everything mocked
- [ ] Integration tests rely on *fast* external services; slower services are mocked
- [ ] Test names clearly describe what's being tested
- [ ] Tests follow conventions (see above)

### Test Organization
- [ ] Tests are organized logically (by feature, by function, etc.)
- [ ] Related test cases are grouped together
- [ ] Test files follow naming and placement conventions (see Test Organization above)

### Placement and naming
- [ ] Unit tests are placed in `tests/` directory or as `*_test.py` files
- [ ] Integration tests are placed in `tests/integration/` or clearly marked
- [ ] Test functions are named `test_<function_name>` or `test_<function_name>_<scenario>`
- [ ] Related test cases are grouped in the same test function or test class

### Assertions
- [ ] Standard `assert` statements are used (pytest enhances these)
- [ ] `assert` messages are clear: `assert result == expected, f"Expected {expected}, got {result}"`
- [ ] `pytest.raises()` is used to test exceptions: `with pytest.raises(ValueError):`
- [ ] `pytest.approx()` is used for floating-point comparisons

### Fixtures
- [ ] `pytest.fixture` is used for test setup and teardown
- [ ] Fixtures are placed in `conftest.py` for shared fixtures
- [ ] Fixture scope is appropriate (`function`, `class`, `module`, `session`)
- [ ] Fixtures clean up after themselves

### Mocking
- [ ] `unittest.mock` or `pytest-mock` is used for mocking
- [ ] `@patch` decorator or `mock.patch()` context manager is used appropriately
- [ ] Mocks are used to isolate units under test
- [ ] Mock assertions verify expected behavior

### Coverage
- [ ] Coverage is calculated using `pytest-cov` or `coverage.py`
- [ ] Coverage reports are reviewed as part of code review
- [ ] `# pragma: no cover` is used sparingly for truly untestable code

### Async Tests
- [ ] `pytest-asyncio` is used for testing async code
- [ ] Async test functions are marked with `@pytest.mark.asyncio`
- [ ] Async fixtures are properly awaited

### Test Organization
- [ ] Test files mirror source structure when possible
- [ ] `conftest.py` is used for shared fixtures and configuration
- [ ] Test utilities are placed in `tests/` or `tests/utils/`
