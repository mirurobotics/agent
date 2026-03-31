
# JavaScript/TypeScript-Specific Stylistic Standards

This file contains JavaScript/TypeScript-specific stylistic recommendations.

## Formatting
- [ ] Code is formatted with `prettier` or follows consistent style guide
- [ ] Line length is 80-100 characters (configurable in prettier/eslint)
- [ ] Semicolons are used consistently (either always or never, per project style)
- [ ] Trailing commas are used in multi-line objects/arrays (helps with git diffs)

## Imports
Imports should be organized logically:

1. External dependencies (node_modules)
2. Internal modules (absolute imports from src/)
3. Relative imports (../ or ./)
4. Type-only imports (when using `import type`)

Each group should be separated by a blank line. Within each group, imports should be sorted alphabetically.

```typescript
// External dependencies
import React from 'react';
import { useQuery } from '@tanstack/react-query';

// Internal modules
import { Button } from '@/components/ui/button';
import { useWorkspace } from '@/features/workspaces/hooks';

// Relative imports
import { helperFunction } from './utils';
import type { Workspace } from '../types';
```

### Import Style
- [ ] ES modules (`import`/`export`) are used instead of CommonJS
- [ ] Named imports are preferred over default imports when possible
- [ ] Type-only imports use `import type` syntax in TypeScript
- [ ] Imports are organized into groups with blank lines
- [ ] Unused imports are removed

## Naming
- [ ] Function and variable names use `camelCase`
- [ ] Class names use `PascalCase`
- [ ] Constant names use `SCREAMING_SNAKE_CASE` or `UPPER_CAMEL_CASE` (project preference)
- [ ] Private class members use leading underscore: `_privateMethod`
- [ ] Component names use `PascalCase` (React convention)
- [ ] File names use `camelCase` for utilities, `PascalCase` for components

## Checklist

### Errors
- [ ] Error variables are named appropriately (e.g. `err`, `error` per project convention)

### Logging
- [ ] No errant print/console statements exist
- [ ] Excessive debug statements do not exist
- [ ] Logging is used appropriately for debugging and monitoring

### Readability
- [ ] Code logic is as vertically compact as reasonably possible
- [ ] Variable names are clear and descriptive
- [ ] Function names clearly describe their purpose
- [ ] Code follows style conventions (see this document)

### Formatting
- [ ] Code is formatted with `prettier` or follows consistent style
- [ ] Line length is enforced (80-100 characters)
- [ ] Semicolons are used consistently
- [ ] Trailing commas are used in multi-line structures

### Imports
- [ ] Imports are organized into groups: external, internal, relative, type-only
- [ ] Each group is separated by a blank line
- [ ] Imports within groups are sorted alphabetically
- [ ] `import type` is used for type-only imports in TypeScript

### Naming
- [ ] Naming conventions are followed (`camelCase`, `PascalCase`, `SCREAMING_SNAKE_CASE`)
- [ ] Private members use leading underscore appropriately
- [ ] Component names use `PascalCase`
- [ ] File names follow project conventions

### Portability
- [ ] No hardcoded `'/'` path separators — use `path.join()`, `path.sep`, or `path.resolve()` instead
- [ ] Path construction uses `path.join()`, not template literals or string concatenation with `"/"`
- [ ] Tests use the same portable path APIs as production code

### Code Style
- [ ] ESLint rules are followed
- [ ] TypeScript strict mode is enabled
- [ ] Code follows JavaScript/TypeScript idioms and best practices
- [ ] Modern JavaScript features are used (ES6+, optional chaining, nullish coalescing)
