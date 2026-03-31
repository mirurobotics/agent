
# Rust-Specific Testing Standards

This file contains Rust-specific testing recommendations.

## Test Organization

Rust tests are organized into:
- **Unit tests**: Placed in the same file as the code they test, in a `#[cfg(test)]` module
- **Integration tests**: Placed in `tests/` directory at the crate root

**Example unit test structure:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name() {
        // test code
    }
}
```

## Test Naming

- Test functions are named `test_<function_name>` or `test_<function_name>_<scenario>`
- Test modules are typically named `tests` or `test_<feature>`

## Parameterized Tests

Rust doesn't have built-in table-driven tests like Go, but similar patterns can be achieved:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name() {
        let test_cases = vec![
            ("scenario 1", InputType { ... }, ExpectedType { ... }),
            ("scenario 2", InputType { ... }, ExpectedType { ... }),
        ];

        for (name, input, expected) in test_cases {
            let actual = function_name(input);
            assert_eq!(actual, expected, "Failed for case: {}", name);
        }
    }
}
```

Or using a macro for more complex cases:
```rust
macro_rules! test_cases {
    ($($name:ident: $input:expr => $expected:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let actual = function_name($input);
                assert_eq!(actual, $expected);
            }
        )*
    };
}

test_cases! {
    test_happy_path: Input { ... } => Expected { ... },
    test_error_case: Input { ... } => Expected { ... },
}
```

## Checklist

### Testability
- [ ] dependencies are injectable (interfaces/traits, function parameters, dependency injection)
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
- [ ] Unit tests are placed in `#[cfg(test)]` modules in the same file
- [ ] Integration tests are placed in `tests/` directory
- [ ] Test functions are named `test_<function_name>` or `test_<function_name>_<scenario>`
- [ ] Related test cases are grouped in the same test module

### Assertions
- [ ] Use standard `assert!`, `assert_eq!`, `assert_ne!` macros
- [ ] Use `assert_eq!(expected, actual)` - **expected value first, actual second**
- [ ] Use `Result` return types for tests that can fail: `#[test] fn test() -> Result<(), Error>`
- [ ] Use `#[should_panic]` attribute for tests that should panic (use sparingly)

### Error Testing
- [ ] Test error cases using `assert!(result.is_err())` or pattern matching
- [ ] Test specific error types when possible: `assert!(matches!(result, Err(ErrorType::Variant)))`

### Coverage
- [ ] Use `cargo tarpaulin` or similar tools for coverage
- [ ] Coverage reports are reviewed as part of code review

### Test Attributes
- [ ] Use `#[ignore]` for slow tests that shouldn't run in normal test suite
- [ ] Use `#[test]` for all test functions
- [ ] Use `#[should_panic(expected = "...")]` when testing panic conditions
