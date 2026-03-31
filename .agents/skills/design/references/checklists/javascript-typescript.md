
# JavaScript/TypeScript-Specific Design Standards

This file contains JavaScript/TypeScript-specific design recommendations.

## Code Organization
- [ ] code is organized logically (by feature, by layer); related functionality is grouped together
- [ ] separation of concerns is maintained
- [ ] module boundaries are well-defined; modules are organized by feature or layer
- [ ] public APIs are minimal (avoid unnecessary exports)
- [ ] internal/private visibility is used appropriately (e.g., non-exported symbols, `private`/protected)

## Naming (Design Principles)
- [ ] names convey intent and purpose clearly (clear and descriptive without being overly verbose)
- [ ] boolean variables/functions read as questions (isValid, hasPrefix, canExecute)

## TypeScript
- [ ] TypeScript is used instead of plain JavaScript when possible
- [ ] types are explicit and not inferred when it improves clarity
- [ ] `any` is avoided; use `unknown` when type is truly unknown
- [ ] `@ts-expect-error` or `@ts-ignore` are avoided; fix type issues instead
- [ ] strict mode is enabled in `tsconfig.json`
- [ ] type definitions are exported for public APIs
- [ ] utility types are used appropriately (`Partial`, `Pick`, `Omit`, `Record`, etc.)

## Error Handling
- [ ] errors are handled explicitly, not ignored (no silent catch blocks)
- [ ] error messages are clear and actionable
- [ ] error types are specific and meaningful (custom error classes for specific cases)
- [ ] custom error classes extend `Error` properly
- [ ] errors are thrown as `Error` instances or custom error classes
- [ ] `try/catch` blocks and promises use `.catch()` or `try/await` for error handling

## Functions
- [ ] functions are focused on a single responsibility
- [ ] function parameters are kept to a reasonable number (typically 3–5 or fewer); prefer an options object for 4 or more
- [ ] functions don't exceed 50 lines; less than 25 lines is heavily preferred; less than 15 is ideal
- [ ] function names aren't overly verbose and aren't redundant to module/component names
- [ ] arrow functions are used for callbacks and when `this` binding is not needed
- [ ] regular functions are used when `this` binding is needed or for hoisting
- [ ] default parameters are used instead of `||` or `??` for function defaults
- [ ] rest parameters (`...args`) are used for variadic functions
- [ ] destructuring is used for function parameters when appropriate
- [ ] async functions return `Promise<T>` explicitly in TypeScript

## Classes and Objects
- [ ] classes are used when state and behavior are tightly coupled
- [ ] functional approaches are preferred when possible (avoid classes for simple data)
- [ ] `private` and `protected` modifiers are used appropriately in TypeScript
- [ ] getters and setters are used sparingly (prefer explicit methods)
- [ ] `readonly` modifier is used for immutable properties in TypeScript
- [ ] object literals are preferred over classes for simple data structures

## Async/Await
- [ ] `async/await` is preferred over promise chains for readability
- [ ] `Promise.all()` is used for parallel async operations
- [ ] `Promise.allSettled()` is used when all promises should complete
- [ ] `Promise.race()` is used appropriately for timeout patterns
- [ ] errors in async functions are properly caught and handled
- [ ] async functions are not used unnecessarily (avoid `async` when not awaiting)

## Promises
- [ ] promises are not created unnecessarily (avoid `new Promise` when async/await works)
- [ ] `.then()` chains are readable and not overly nested
- [ ] `Promise.resolve()` and `Promise.reject()` are used appropriately

## Destructuring
- [ ] destructuring is used for object and array access when it improves readability
- [ ] default values are provided in destructuring: `const { name = 'default' } = obj`
- [ ] rest operator is used in destructuring: `const { a, b, ...rest } = obj`

## Modules
- [ ] ES modules (`import`/`export`) are used instead of CommonJS (`require`/`module.exports`)
- [ ] named exports are preferred over default exports when possible
- [ ] barrel exports (`index.ts`) are used appropriately (not overused)
- [ ] circular dependencies are avoided
- [ ] imports are organized logically (external, internal, relative)

## React-Specific (when using React)
- [ ] functional components are used instead of class components
- [ ] hooks follow the Rules of Hooks (only at top level, not in conditionals)
- [ ] custom hooks are used for reusable logic
- [ ] `useMemo` and `useCallback` are used only when necessary (not overused)
- [ ] props are typed with TypeScript interfaces or types
- [ ] component props are destructured in function parameters

## State Management
- [ ] state is lifted to the appropriate level (not too high, not too low)
- [ ] state updates are immutable (use spread, `map`, `filter`, etc.)
- [ ] state management library (Redux, Zustand, etc.) is used when local state is insufficient
- [ ] state is normalized when dealing with complex data structures

## Type Safety
- [ ] type assertions (`as`) are avoided; use type guards instead
- [ ] type guards are used for narrowing types: `function isType(value): value is Type`
- [ ] discriminated unions are used for type-safe state machines
- [ ] `null` and `undefined` are handled explicitly (use `Optional<T>` or `T | null | undefined`)

## Collections
- [ ] appropriate collection type is chosen (`Array`, `Set`, `Map`, `Object`)
- [ ] `Set` is used for unique values
- [ ] `Map` is used for key-value pairs when keys are not strings
- [ ] array methods are preferred over manual loops when readable

## Event Handling
- [ ] event handlers are properly typed in TypeScript
- [ ] event handlers are debounced/throttled when appropriate
