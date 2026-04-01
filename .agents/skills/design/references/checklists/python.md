
# Python-Specific Design Standards

This file contains Python-specific design recommendations.

## Code Organization
- [ ] code is organized logically (by feature, by layer); related functionality is grouped together
- [ ] separation of concerns is maintained
- [ ] module boundaries are well-defined; modules are organized by feature or layer
- [ ] public APIs are minimal (use `__all__`; avoid unnecessary public names)
- [ ] internal visibility is used appropriately (leading underscore for module-private)

## Naming (Design Principles)
- [ ] names convey intent and purpose clearly (clear and descriptive without being overly verbose)
- [ ] boolean variables/functions read as questions (is_valid, has_prefix, can_execute)

## Type Hints
- [ ] type hints are used for function parameters and return values (PEP 484, PEP 526)
- [ ] `typing` module is used for complex types (`List`, `Dict`, `Optional`, `Union`, etc.)
- [ ] Python 3.9+ style type hints are preferred when available (`list[str]` instead of `List[str]`)
- [ ] `Optional[T]` or `T | None` is used for nullable types
- [ ] `Protocol` is used for structural typing when appropriate
- [ ] `TypedDict` is used for dictionary structures with known keys

## Error Handling
- [ ] errors are handled explicitly, not ignored (exceptions are used instead of error codes or return values)
- [ ] error messages are clear and actionable
- [ ] error types are specific and meaningful (specific exception types, not generic `Exception`)
- [ ] custom exception classes inherit from appropriate base exceptions for specific error cases
- [ ] exceptions are caught at the appropriate level (don't catch everything)
- [ ] `try/except/else/finally` blocks are used appropriately
- [ ] exception chaining is used when re-raising: `raise NewError from original_error`

## Functions
- [ ] functions are focused on a single responsibility
- [ ] function parameters are kept to a reasonable number (typically 3–5 or fewer); prefer a dataclass or options object for 4 or more
- [ ] functions don't exceed 50 lines; less than 25 lines is heavily preferred; less than 15 is ideal
- [ ] function names aren't overly verbose and aren't redundant to module names
- [ ] keyword-only arguments are used for optional parameters: `def func(required, *, optional=None)`
- [ ] default parameter values are immutable (avoid mutable defaults like `[]` or `{}`)
- [ ] `*args` and `**kwargs` are used appropriately for variadic functions
- [ ] function decorators are used for cross-cutting concerns (logging, caching, etc.)

## Classes and Objects
- [ ] classes follow single responsibility principle
- [ ] `dataclasses` or `@dataclass` are used for simple data containers (Python 3.7+)
- [ ] `__slots__` is considered for classes with many instances to reduce memory
- [ ] properties are used instead of getters/setters when appropriate
- [ ] `__init__` is kept simple; use factory methods or builders for complex initialization
- [ ] magic methods (`__str__`, `__repr__`, `__eq__`, etc.) are implemented when appropriate

## Context Managers
- [ ] context managers (`with` statements) are used for resource management
- [ ] `contextlib.contextmanager` is used for simple context managers
- [ ] `contextlib.ExitStack` is used when managing multiple resources
- [ ] custom context managers implement `__enter__` and `__exit__`

## Async/Await
- [ ] `async`/`await` is used for I/O-bound operations
- [ ] `asyncio` is used appropriately (not overused for CPU-bound tasks)
- [ ] async context managers (`async with`) are used for async resources
- [ ] `asyncio.gather()` or `asyncio.create_task()` is used for concurrent async operations
- [ ] proper error handling in async code (exceptions in tasks are handled)

## Comprehensions and Generators
- [ ] list/dict/set comprehensions are used when readable (prefer over loops for simple transformations)
- [ ] generator expressions are used for large sequences to save memory
- [ ] generators (`yield`) are used for lazy evaluation when appropriate
- [ ] comprehensions are not overused (complex logic may be clearer in a loop)

## Imports and Modules
- [ ] absolute imports are preferred over relative imports
- [ ] `__all__` is used to define public API of modules
- [ ] circular imports are avoided
- [ ] imports are organized: standard library, third-party, local (PEP 8)

## Type Safety
- [ ] type guards or assertions are used when narrowing types
- [ ] `isinstance()` is preferred over type checking with `type()`
- [ ] `Enum` is used for constants instead of magic strings/numbers

## Collections
- [ ] appropriate collection type is chosen (`list`, `tuple`, `set`, `dict`, `deque`)
- [ ] `tuple` is used for immutable sequences
- [ ] `frozenset` is used for immutable sets
- [ ] `collections.namedtuple` or `dataclass` is used for structured data

## Decorators
- [ ] decorators are used appropriately (logging, caching, validation, etc.)
- [ ] `functools.wraps` is used in custom decorators to preserve metadata
- [ ] decorator composition is clear and readable
