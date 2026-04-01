
# JavaScript/TypeScript-Specific Testing Standards

This file contains JavaScript/TypeScript-specific testing recommendations.

## Test Organization

JavaScript/TypeScript tests are typically organized as:
- **Unit tests**: Placed alongside code as `*.test.ts`/`*.test.tsx` or in `__tests__/` directories
- **Integration tests**: Placed in `tests/` or `__tests__/integration/` directories
- **Test files**: Named `*.test.ts`, `*.test.tsx`, `*.spec.ts`, or `*.spec.tsx`

## Test Framework

- [ ] `vitest` or `jest` is used as the testing framework
- [ ] `@testing-library/react` is used for React component testing
- [ ] `@testing-library/user-event` is used for user interaction testing
- [ ] Test utilities and helpers are organized in `test/` or `__tests__/utils/`

## Parameterized Tests

JavaScript/TypeScript uses `test.each` or `describe.each` for parameterized tests:

```typescript
import { describe, it, expect } from 'vitest';

describe('functionName', () => {
    it.each([
        { name: 'happy path', input: Input(...), expected: Expected(...) },
        { name: 'error case', input: Input(...), expectError: true },
    ])('should handle $name', ({ input, expected, expectError }) => {
        if (expectError) {
            expect(() => functionName(input)).toThrow();
        } else {
            expect(functionName(input)).toEqual(expected);
        }
    });
});
```

Or using `describe.each` for multiple test cases:

```typescript
describe.each([
    { input: 'hello', expected: 'HELLO' },
    { input: 'world', expected: 'WORLD' },
    { input: '', expected: '' },
])('uppercase($input)', ({ input, expected }) => {
    it(`should return ${expected}`, () => {
        expect(input.toUpperCase()).toBe(expected);
    });
});
```

## Test Naming

- [ ] Test functions are named descriptively: `it('should do something', ...)`
- [ ] Test suites use `describe()` blocks to group related tests
- [ ] Test descriptions are clear and describe the behavior being tested

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
- [ ] Unit tests are placed alongside code as `*.test.ts` or in `__tests__/` directories
- [ ] Integration tests are placed in `tests/` or `__tests__/integration/` directories
- [ ] Test files are named `*.test.ts`, `*.test.tsx`, `*.spec.ts`, or `*.spec.tsx`
- [ ] Related test cases are grouped in `describe()` blocks

### Assertions
- [ ] Matcher functions are used appropriately (`toBe`, `toEqual`, `toContain`, etc.)
- [ ] `expect().toEqual()` is used for object/array comparisons
- [ ] `expect().toBe()` is used for primitive value comparisons (reference equality)
- [ ] `expect().toThrow()` is used to test exceptions
- [ ] Custom matchers are used when helpful (e.g., `toHaveBeenCalledWith`)

### Mocking
- [ ] `vi.fn()` (vitest) or `jest.fn()` (jest) is used for creating mocks
- [ ] `vi.spyOn()` or `jest.spyOn()` is used for spying on functions
- [ ] `vi.mock()` or `jest.mock()` is used for module mocking
- [ ] Mocks are reset between tests (`beforeEach`, `afterEach`)
- [ ] Mock implementations are clear and test the right behavior

### React Testing
- [ ] `@testing-library/react` is used for rendering components
- [ ] `render()` is used to render components in tests
- [ ] Queries are used to find elements (`getByRole`, `getByText`, `queryBy`, etc.)
- [ ] User interactions are tested with `@testing-library/user-event`
- [ ] Components are tested from the user's perspective (not implementation details)

### Async Testing
- [ ] `async/await` is used in test functions when testing async code
- [ ] `waitFor()` is used when waiting for async updates
- [ ] `findBy*` queries are used for elements that appear asynchronously
- [ ] Promise rejections are properly tested

### Coverage
- [ ] Coverage is calculated using `vitest --coverage` or `jest --coverage`
- [ ] Coverage reports are reviewed as part of code review
- [ ] `/* istanbul ignore */` or `/* c8 ignore */` comments are used sparingly

### Test Utilities
- [ ] Test utilities are placed in `test/` or `__tests__/utils/`
- [ ] Custom render functions wrap `@testing-library/react` render when needed
- [ ] Test fixtures and factories are used for creating test data
- [ ] `setupTests.ts` or similar is used for global test configuration
