
# Rust-Specific Stylistic Standards

This file contains Rust-specific stylistic recommendations.

## Formatting
- [ ] Code is formatted with `rustfmt` (typically via `cargo fmt`)
- [ ] `rustfmt.toml` configuration is used consistently across the project
- [ ] Maximum line length is typically 100 characters (configurable in `rustfmt.toml`)

## Imports
Rust imports (use statements) should be organized logically:

1. Standard library imports
2. External crate imports
3. Internal crate imports (current crate)
4. Super/parent module imports
5. Self imports

**Example:**
```rust
// Standard library
use std::collections::HashMap;
use std::path::Path;

// External crates
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

// Current crate
use crate::models::User;
use crate::utils::helpers;

// Super module
use super::types;

// Self
use self::inner::InnerType;
```

Alternatively, group by module path:
```rust
use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::models::User;
use crate::utils::helpers;
```

## Naming
- [ ] Function names use `snake_case`
- [ ] Type names use `PascalCase`
- [ ] Constant names use `SCREAMING_SNAKE_CASE`
- [ ] Module names use `snake_case`
- [ ] Lifetimes use short, descriptive names (`'a`, `'ctx`, `'static`)

## Checklist

### Errors
- [ ] Error variables are named appropriately (e.g. `e` or `err` per project convention)

### Logging
- [ ] No errant print/eprint statements exist
- [ ] Excessive debug statements do not exist
- [ ] Logging is used appropriately for debugging and monitoring

### Readability
- [ ] Code logic is as vertically compact as reasonably possible
- [ ] Variable names are clear and descriptive
- [ ] Function names clearly describe their purpose
- [ ] Code follows style conventions (see this document)

### Formatting
- [ ] Code is formatted with `cargo fmt` or `rustfmt`
- [ ] `rustfmt.toml` is configured consistently
- [ ] Line length is enforced (typically 100 characters)

### Imports
- [ ] Imports are organized logically (standard library, external, internal)
- [ ] Unused imports are removed (enforced by compiler)
- [ ] Import groups are separated by blank lines when helpful

### Modules
- [ ] `pub(crate)` or `pub(super)` is used for internal visibility
- [ ] Module file organization follows Rust conventions. For logical structure and boundaries, see design checklists (Code Organization).

### Portability
- [ ] No hardcoded `'/'` path separators — use `std::path::MAIN_SEPARATOR`, `Path::join()`, or `PathBuf` methods instead
- [ ] Path construction uses `Path`/`PathBuf` APIs, not string formatting with `"/"`
- [ ] Tests use the same portable path APIs as production code

### Code Style
- [ ] Naming conventions are followed (`snake_case`, `PascalCase`, `SCREAMING_SNAKE_CASE`)
- [ ] Clippy warnings are addressed (when using `clippy`)
- [ ] Code follows Rust idioms and best practices
