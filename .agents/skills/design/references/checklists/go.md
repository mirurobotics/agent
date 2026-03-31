
# Go-Specific Design Standards

This file contains Go-specific design recommendations.

## Concurrency
- [ ] goroutines have clear ownership and termination conditions
- [ ] channels are closed by senders, not receivers
- [ ] shared mutable state is avoided entirely or at least protected by sync primitives
- [ ] context cancellation is propagated and respected in goroutines
- [ ] sync.Mutex guards are held for the minimum time necessary
- [ ] `sync.WaitGroup` is used to wait for goroutines to complete
- [ ] buffered channels are used when producer/consumer rates differ significantly

## Context
- [ ] context is passed as the first argument to functions doing I/O or long-running tasks
- [ ] context is not passed to functions unrelated to I/O or long-running tasks
- [ ] context values are used for request-scoped data, not for passing optional parameters
- [ ] `context.Background()` or `context.TODO()` are only used at the top level (main, tests, handlers)

## Error Handling
- [ ] errors are handled explicitly, not ignored
- [ ] error messages are clear and actionable (start with lowercase, don't end with punctuation)
- [ ] error types are specific and meaningful (avoid generic string errors)
- [ ] custom error types implement the `error` interface; sentinel errors are used for expected errors
- [ ] `errors.Is()` and `errors.As()` are used for error checking instead of type assertions
- [ ] errors are wrapped with context using `fmt.Errorf("...: %w", err)` or `errors.Wrap()`

## Functions
- [ ] functions are focused on a single responsibility
- [ ] function parameters are kept to a reasonable number (typically 3–5 or fewer); prefer a struct for 4 or more parameters
- [ ] functions don't exceed 50 lines; less than 25 lines is heavily preferred; less than 15 is ideal
- [ ] function names aren't overly verbose and aren't redundant to package names
- [ ] functional options are used for optional function arguments when there are 3+ optional parameters
- [ ] function parameters are ordered: context, inputs, outputs (context first, then inputs, then outputs)
- [ ] variadic functions are used appropriately (e.g., `fmt.Printf`, not for optional parameters)

**Functional Options Pattern Example:**
```go
type Option func(*Config)

func WithTimeout(timeout time.Duration) Option {
    return func(c *Config) {
        c.Timeout = timeout
    }
}

func NewClient(opts ...Option) *Client {
    cfg := defaultConfig()
    for _, opt := range opts {
        opt(&cfg)
    }
    return &Client{config: cfg}
}
```

## Interfaces
- [ ] interfaces are small (1-3 methods preferred)
- [ ] interfaces are defined where they're consumed, not where implemented
- [ ] functions accept interfaces, return concrete types
- [ ] `any`/`interface{}` is avoided unless truly necessary (e.g., reflection, generic constraints)
- [ ] interface names typically end with `-er` when representing a single action (e.g., `Reader`, `Writer`)

## Method Receivers
- [ ] receiver names are short (1-2 letters) and consistent across methods
- [ ] pointer receivers (`*T`) are used when:
  - method needs to modify the receiver
  - struct is large (to avoid copying)
  - consistency (if any method uses pointer receiver, all should)
- [ ] value receivers (`T`) are used for small, immutable types


## Code Organization
- [ ] code is organized logically (by feature, by layer); related functionality is grouped together
- [ ] separation of concerns is maintained
- [ ] package boundaries are well-defined; packages are organized by feature or layer
- [ ] public APIs are minimal (avoid unnecessary exported names)
- [ ] unexported (lowercase) visibility is used for internal implementation

## Naming (Design Principles)
- [ ] names convey intent and purpose clearly (clear and descriptive without being overly verbose)
- [ ] boolean variables/functions read as questions (isValid, hasPrefix, canExecute)

## Packages
- [ ] package names are short, lowercase, single-word (no underscores or mixedCaps)
- [ ] package names are based on what they provide, not what they contain
- [ ] `internal/` directory is used for packages that should not be imported by other projects
- [ ] `init()` functions are avoided unless necessary (e.g., registering with a global registry)

## Structs and Embedding
- [ ] struct fields are organized logically (related fields together)
- [ ] embedding is used for composition, not just to save typing
- [ ] embedded types are used to model "is-a" relationships
- [ ] exported struct fields are used when the struct should be used as a value type

## Type Safety
- [ ] custom types are used for domain concepts (e.g., `type UserID int64`)
- [ ] type assertions use comma-ok idiom: `val, ok := x.(Type)`
- [ ] type switches are used when checking multiple types
- [ ] type aliases (`type A = B`) are avoided; use type definitions (`type A B`) for new types

## Zero Values
- [ ] nil slices/maps are handled gracefully where received
- [ ] zero values are useful (e.g., empty slice, zero int, empty string)
- [ ] structs are designed so zero value is useful (avoid requiring initialization)

## Generics (Go 1.18+)
- [ ] generics are used when they improve type safety or reduce code duplication
- [ ] generic constraints are clear and minimal
- [ ] type parameters have descriptive names (e.g., `T`, `K`, `V` for common cases)
- [ ] generics are not overused (prefer interfaces when appropriate)
