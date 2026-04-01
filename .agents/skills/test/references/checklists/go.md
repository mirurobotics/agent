
# Go-Specific Testing Standards

This file contains Go-specific testing recommendations.

## Test Placement

| Test Type | Location | 
|-----------|----------|
| **Unit tests** | `pkg/<name>/<name>_test.go` or `internal/<name>/<name>_test.go` |
| **Integration tests** | `tests/<name>/<name>_test.go` |

## Table-Driven Tests

Table-driven tests are the preferred way to test many cases for a given function. Below is a template you can follow for writing table-driven tests.

```go
import (
    "testing"

    "github.com/mirurobotics/core/pkg/assert"
)

func TestFunctionName(t *testing.T) {
    tests := []struct {
        name      string
        input     InputType
        expected  OutputType
        expectErr bool
    }{
        {
            name:     "descriptive name of scenario",
            input:    InputType{...},
            expected: OutputType{...},
        },
        {
            name:      "error case description",
            input:     InputType{...},
            expectErr: true,
        },
    }

    for _, tt := range tests {
        t.Run(tt.name, func(t *testing.T) {
            actual, err := FunctionName(tt.input)
            if tt.expectErr {
                assert.Error(t, err)
                return
            }
            assert.NoError(t, err)
            assert.Equal(t, tt.expected, actual) // expected first, actual second
        })
    }
}
```

## Test Naming

- Test functions are named `Test<FunctionName>` or `Test<FunctionName>_<Scenario>`
- Subtests use descriptive names passed to `t.Run()`

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
- [ ] Test files follow naming and placement conventions (see Test Placement above)

### Table-driven and naming
- [ ] Use table-driven tests for functions with multiple input scenarios
- [ ] Name test functions as `Test<FunctionName>` or `Test<FunctionName>_<Scenario>`
- [ ] Group related test cases logically
- [ ] Use subtests with `t.Run()` for table-driven tests

### Assertions
- [ ] Use the `pkg/assert` package for all assertions (or similar assertion library)
- [ ] Use `assert.Equal(t, expected, actual)` - **expected value first, actual second**
- [ ] Use `assert.NoError(t, err)` and `assert.Error(t, err)` for error checks
- [ ] Use `assert.Nil(t, val)` and `assert.NotNil(t, val)` for nil checks

### Coverage
- [ ] Calculate coverage using `go test -cover` or project-specific coverage scripts
- [ ] Coverage reports are reviewed as part of code review
