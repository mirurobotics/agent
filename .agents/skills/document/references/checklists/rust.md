
# Rust-Specific Documentation Standards

This file contains Rust-specific documentation recommendations. 

## Module Documentation
Every public module should have documentation explaining its purpose. Module documentation is written using `//!` (inner doc comments) at the top of the module file or using `///` (outer doc comments) on the `mod` declaration.

Module documentation should:
- Explain the module's purpose and responsibility
- Provide usage examples if the module has a primary API
- Reference external resources when relevant (e.g., third-party service documentation)

### Checklist
- [ ] Public modules have documentation explaining their purpose
- [ ] Module documentation includes usage examples if applicable
- [ ] Module documentation references external resources when relevant

## Doc Comments
Rust uses doc comments (`///` for items, `//!` for modules) that are processed by `rustdoc`. Doc comments support Markdown and are rendered as HTML documentation.

**Conventions:**
- Doc comments use complete sentences
- The first line should be a brief summary
- Use `# Examples` section for code examples
- Use `# Panics` section to document when functions panic
- Use `# Errors` section to document error conditions
- Use `# Safety` section for `unsafe` functions

**Example:**
```rust
/// Processes a single item and returns an error if processing fails.
///
/// The item must be in a valid state before processing. This function is
/// safe to call concurrently.
///
/// # Examples
///
/// ```
/// let item = Item::new();
/// process_item(&item)?;
/// ```
///
/// # Errors
///
/// Returns an error if the item is in an invalid state.
pub fn process_item(item: &Item) -> Result<(), Error> {
    // ...
}
```

### Checklist
- [ ] docstrings are omitted if the code is sufficiently self-explanatory; docstrings are included if the code has unguessable or unintuitive side-effects
- [ ] docstrings are only used for private components if particularly unintuitive
- [ ] docstrings give more information than can be easily guessed by the function's name
- [ ] Public items (functions, structs, enums, traits, etc.) have doc comments
- [ ] Doc comments use complete sentences
- [ ] First line is a brief summary
- [ ] Code examples are included in `# Examples` sections when helpful
- [ ] Panic conditions are documented in `# Panics` sections
- [ ] Error conditions are documented in `# Errors` sections
- [ ] `unsafe` functions have `# Safety` sections explaining invariants

## Doc Test Examples
Rust doc comments can include code examples that are automatically tested. These examples:
- Are placed in `///` doc comments
- Are wrapped in code fences (```)
- Are compiled and run as part of the test suite
- Can use `#` to hide setup code from the rendered documentation

**Example:**
```rust
/// Creates a new client with default settings.
///
/// # Examples
///
/// ```
/// # use my_crate::Client;
/// let client = Client::new();
/// ```
pub fn new() -> Client {
    // ...
}
```

### Checklist
- [ ] examples are clear and demonstrate the documented feature
- [ ] examples don't include unnecessary complexity
- [ ] examples are kept up to date with API changes
- [ ] Doc test examples are included for public APIs
- [ ] Doc test examples are complete and runnable
- [ ] Setup code is hidden using `#` when appropriate
- [ ] Examples demonstrate common usage patterns

## Inline comments
Inline comments in function bodies explain unintuitive parts of the code, not what the code is doing.

### Checklist
- [ ] inline comments do not explain what the code is doing (the code should do this)
- [ ] inline comments are used to explain unintuitive sections or algorithms
- [ ] complex algorithms or business logic have brief comments explaining the "why" not the "what"

## Inline Documentation (struct fields, enum variants)
Rust also supports inline documentation for struct fields and enum variants using doc comments.

**Example:**
```rust
pub struct Config {
    /// The maximum number of concurrent connections.
    /// Defaults to 100 if not specified.
    pub max_connections: usize,
    
    /// Timeout in seconds for network operations.
    pub timeout: u64,
}
```

### Checklist
- [ ] Public struct fields have doc comments explaining their purpose
- [ ] Public enum variants have doc comments when their purpose isn't obvious
- [ ] Default values are documented when applicable
