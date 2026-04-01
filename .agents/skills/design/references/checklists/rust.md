
# Rust-Specific Design Standards

This file contains Rust-specific design recommendations.

## Code Organization
- [ ] code is organized logically (by feature, by layer); related functionality is grouped together
- [ ] separation of concerns is maintained
- [ ] module boundaries are well-defined; modules are organized by feature or layer
- [ ] public APIs are minimal (avoid unnecessary `pub` exports)
- [ ] visibility (`pub`, `pub(crate)`, private) is used appropriately

## Naming (Design Principles)
- [ ] names convey intent and purpose clearly (clear and descriptive without being overly verbose)
- [ ] boolean variables/functions read as questions (is_valid, has_prefix, can_execute)

## Ownership and Borrowing
- [ ] ownership is clear and explicit
- [ ] borrowing rules are respected (no unnecessary cloning)
- [ ] `clone()` is only used when necessary (avoid unnecessary allocations)
- [ ] references are preferred over owned values when possible
- [ ] `Rc`/`Arc` are used only when shared ownership is truly needed

## Error Handling
- [ ] errors are handled explicitly, not ignored (use `Result`/`Option`, avoid silent `let _ =`)
- [ ] error messages are clear and actionable
- [ ] error types are specific and meaningful (custom error types for specific cases)
- [ ] custom error types implement `std::error::Error` trait
- [ ] `Result<T, E>` is used for fallible operations; `Option<T>` for nullable/optional values
- [ ] `?` operator is used for error propagation when appropriate
- [ ] `unwrap()` and `expect()` are avoided in production code (use only in tests or when panic is acceptable)

## Concurrency
- [ ] `Send` and `Sync` traits are understood and respected
- [ ] `Arc` is used for shared ownership across threads (not `Rc`)
- [ ] `Mutex`/`RwLock` guards are held for the minimum time necessary
- [ ] async/await is used appropriately (not overused for simple operations)
- [ ] `tokio::spawn` (or similar) has clear ownership, cancellation, and error handling
- [ ] channels (`mpsc`, `tokio::sync::mpsc`) are used for communication between tasks

## Functions
- [ ] functions are focused on a single responsibility
- [ ] function parameters are kept to a reasonable number (typically 3–5 or fewer); prefer a struct for 4 or more
- [ ] functions don't exceed 50 lines; less than 25 lines is heavily preferred; less than 15 is ideal
- [ ] function names aren't overly verbose and aren't redundant to module names
- [ ] for functions with many optional parameters, prefer:
  - `Default` trait for optional parameters with sensible defaults
  - struct initialization (pass a config struct instead of many parameters)
  - builder pattern only when Default doesn't work well (e.g., validation required, no good defaults)

## Traits
- [ ] traits are small and focused (1-3 methods preferred)
- [ ] traits are defined where they're consumed, not where implemented
- [ ] generic constraints are clear and minimal
- [ ] trait bounds are used appropriately (avoid unnecessary bounds)
- [ ] `dyn Trait` is used when dynamic dispatch is needed, generics when static dispatch is sufficient


## Type Safety
- [ ] newtype pattern is used for domain concepts (e.g., `struct UserId(u64)`)
- [ ] enums are used for sum types (instead of magic numbers or strings)
- [ ] `Option<T>` is used instead of nullable pointers
- [ ] `Result<T, E>` is used instead of exceptions or error codes
- [ ] pattern matching is exhaustive (use `match` instead of `if let` when all cases should be handled)

## Memory Safety
- [ ] unsafe code is avoided unless absolutely necessary
- [ ] when `unsafe` is used, invariants are clearly documented
- [ ] unsafe blocks are as small as possible
- [ ] raw pointers are avoided in favor of references or smart pointers

## Collections
- [ ] appropriate collection type is chosen (`Vec`, `HashMap`, `HashSet`, `BTreeMap`, etc.)
- [ ] `HashMap` vs `BTreeMap` choice is based on key ordering needs
- [ ] `Vec` vs array choice is based on size mutability needs

## Async/Await
- [ ] async functions are only used when I/O or concurrency is needed
- [ ] `.await` points are clearly identified
- [ ] cancellation tokens or channels are used for graceful shutdown
