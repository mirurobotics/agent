
# Go-Specific Stylistic Standards

This file contains Go-specific stylistic recommendations.

## Formatting
- [ ] Code is formatted with `gofmt` or `goimports`
- [ ] `goimports` is used to automatically organize imports
- [ ] Maximum line length is 88-100 characters

## Imports
Imports should be separated into three different sections:

1. Standard Go package imports
2. Internal Go package imports
3. Third-party Go package imports

Each section should be a single block with a single new line in between each section:

```go
import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"strings"

	errs "github.com/mirurobotics/core/pkg/errs"
	"github.com/mirurobotics/core/pkg/rand"

	"github.com/mr-tron/base58"
)
```

Of course, any section may be omitted if there are no imports for that section.

## Naming
- [ ] Package names are short and unbroken (no underscores between words)
- [ ] Exported names are capitalized, unexported names are lowercase
- [ ] Acronyms are all caps in exported names (e.g., `URL`, `ID`, `HTTP`)
- [ ] Function names follow Go conventions (exported/unexported based on capitalization)

## Checklist

### Errors
- [ ] Error variables are named appropriately (e.g. `err` for error return values)

### Logging
- [ ] No errant print statements exist
- [ ] Excessive debug statements do not exist
- [ ] Logging is used appropriately for debugging and monitoring

### Readability
- [ ] Code logic is as vertically compact as reasonably possible
- [ ] Variable names are clear and descriptive
- [ ] Function names clearly describe their purpose
- [ ] Code follows style conventions (see this document)

### Imports
- [ ] Imports are organized into three sections: standard library, internal, third-party
- [ ] Each section is a single block with blank lines between sections
- [ ] `goimports` is used to automatically format imports

### Packages
- [ ] Package names are short and unbroken (no underscores between words)
- [ ] Package organization follows Go conventions (flat structure preferred). For logical structure and boundaries, see design checklists (Code Organization).

### Portability
- [ ] No hardcoded `'/'` path separators — use `filepath.Join()`, `filepath.Separator`, or `path/filepath` functions instead
- [ ] Path construction uses `filepath.Join()`, not `fmt.Sprintf` or string concatenation with `"/"`
- [ ] Tests use the same portable path APIs as production code

### Code Style
- [ ] Code follows `gofmt` formatting
- [ ] Line length is enforced (88-100 characters)
- [ ] Naming conventions are followed (exported/unexported, acronyms)
