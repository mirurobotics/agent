
# JavaScript/TypeScript-Specific Documentation Standards

This file contains JavaScript/TypeScript-specific documentation recommendations.

## Module Documentation
TypeScript/JavaScript modules can have JSDoc comments at the top of the file explaining the module's purpose. For TypeScript, module-level documentation is less common than in other languages, but can be useful for complex modules.

### Checklist
- [ ] Complex modules have JSDoc comments explaining their purpose
- [ ] Module documentation includes usage examples if applicable

## JSDoc Comments
JavaScript/TypeScript uses JSDoc comments for documentation. JSDoc comments use `/** */` syntax and support type information and structured documentation.

**Conventions:**
- JSDoc comments use `/** */` (not `//`)
- First line is a brief summary
- `@param` tags document parameters
- `@returns` or `@return` documents return value
- `@throws` or `@throws` documents exceptions
- `@example` provides usage examples

**Example:**
```typescript
/**
 * Processes a single item and returns the result.
 * 
 * The item must be in a valid state before processing. This function is
 * safe to call concurrently.
 * 
 * @param item - The item to process. Must be in a valid state.
 * @returns A Result object containing the processing outcome.
 * @throws {ValueError} If the item is in an invalid state.
 * @throws {ProcessingError} If processing fails.
 * 
 * @example
 * ```ts
 * const item = new Item({ id: '123' });
 * const result = processItem(item);
 * ```
 */
function processItem(item: Item): Result {
    // ...
}
```

### Checklist
- [ ] docstrings are omitted if the code is sufficiently self-explanatory; docstrings are included if the code has unguessable or unintuitive side-effects
- [ ] docstrings are only used for private components if particularly unintuitive
- [ ] docstrings give more information than can be easily guessed by the function's name
- [ ] Public functions, classes, and methods have JSDoc comments
- [ ] JSDoc comments use `/** */` syntax
- [ ] First line is a brief summary
- [ ] `@param` tags document all parameters with types
- [ ] `@returns` tag documents return value with type
- [ ] `@throws` tags document exceptions
- [ ] `@example` sections provide usage examples when helpful

## Type Hints in Documentation
In TypeScript, type information in function signatures serves as primary documentation. JSDoc should complement, not duplicate, type information.

### Checklist
- [ ] Type annotations are present in function signatures (TypeScript)
- [ ] JSDoc adds context beyond what types provide
- [ ] Complex types are explained in JSDoc even if type annotations exist

## Inline Documentation
JavaScript/TypeScript also supports inline comments.

### Checklist
- [ ] inline comments do not explain what the code is doing (the code should do this)
- [ ] inline comments are used to explain unintuitive sections or algorithms (the "why" not the "what")
- [ ] `// TODO:` and `// FIXME:` comments are used appropriately

## Examples
JSDoc supports `@example` tags for usage examples. These examples:
- Are placed in JSDoc comments
- Should be runnable and demonstrate common usage
- Can include TypeScript code examples

**Example:**
```typescript
/**
 * Calculates the total of an array of numbers.
 * 
 * @param numbers - Array of numbers to sum
 * @returns The sum of all numbers in the array
 * 
 * @example
 * ```ts
 * calculateTotal([1, 2, 3]) // returns 6
 * calculateTotal([]) // returns 0
 * ```
 */
function calculateTotal(numbers: number[]): number {
    return numbers.reduce((sum, n) => sum + n, 0);
}
```

### Checklist
- [ ] examples are clear and demonstrate the documented feature
- [ ] examples don't include unnecessary complexity
- [ ] examples are kept up to date with API changes
- [ ] Examples are included in JSDoc for complex or non-obvious APIs
- [ ] Examples are runnable and demonstrate common usage patterns
- [ ] Examples use TypeScript syntax when applicable

## React Component Documentation
React components should have JSDoc comments explaining their purpose and props.

**Example:**
```typescript
/**
 * A button component that displays a workspace logo.
 * 
 * @param workspace - The workspace object containing logo information
 * @param size - The size of the logo (default: 'md')
 * @param onClick - Callback function when button is clicked
 */
export function WorkspaceLogoButton({
    workspace,
    size = 'md',
    onClick,
}: WorkspaceLogoButtonProps) {
    // ...
}
```

### Checklist
- [ ] React components have JSDoc comments
- [ ] Component props are documented in JSDoc
- [ ] Complex component behavior is explained
