
# Python-Specific Stylistic Standards

This file contains Python-specific stylistic recommendations.

## Formatting
- [ ] Code is formatted with `black` or follows PEP 8
- [ ] Line length is 88-100 characters (black default is 88)
- [ ] `isort` is used to organize imports
- [ ] `ruff` or `flake8` is used for linting

## Imports
Imports should be organized according to PEP 8:

1. Standard library imports
2. Related third-party imports
3. Local application/library specific imports

Each group should be separated by a blank line. Within each group, imports should be sorted alphabetically.

```python
# Standard library
import os
import sys
from typing import List, Optional

# Third-party
import requests
from pydantic import BaseModel

# Local
from myapp.models import User
from myapp.utils import helpers
```

### Import Style
- [ ] Absolute imports are preferred over relative imports
- [ ] Wildcard imports (`from module import *`) are avoided
- [ ] Imports are organized into three groups with blank lines
- [ ] `isort` is used to automatically format imports

## Naming
- [ ] Function and variable names use `snake_case`
- [ ] Class names use `PascalCase`
- [ ] Constant names use `SCREAMING_SNAKE_CASE`
- [ ] Private names (internal to module/class) use leading underscore: `_private`
- [ ] Name mangling (double underscore) is avoided unless necessary: `__name`
- [ ] Module names use `snake_case` and are short

## Checklist

### Errors
- [ ] Error variables are named appropriately (e.g. `e` or `err` per project convention)

### Logging
- [ ] No errant print statements exist
- [ ] Excessive debug statements do not exist
- [ ] Logging is used appropriately for debugging and monitoring

### Readability
- [ ] Code logic is as vertically compact as reasonably possible
- [ ] Variable names are clear and descriptive
- [ ] Function names clearly describe their purpose
- [ ] Code follows style conventions (see this document)

### Formatting
- [ ] Code is formatted with `black` or follows PEP 8
- [ ] Line length is enforced (88-100 characters)
- [ ] `isort` is used to organize imports
- [ ] Linting tools (`ruff`, `flake8`, `pylint`) are used

### Imports
- [ ] Imports are organized into three groups: standard library, third-party, local
- [ ] Each group is separated by a blank line
- [ ] Imports within groups are sorted alphabetically
- [ ] `isort` is used to automatically format imports

### Naming
- [ ] Naming conventions are followed (`snake_case`, `PascalCase`, `SCREAMING_SNAKE_CASE`)
- [ ] Private names use leading underscore appropriately
- [ ] Module names are short and use `snake_case`

### Portability
- [ ] No hardcoded `'/'` path separators — use `os.path.join()`, `pathlib.Path`, or `os.sep` instead
- [ ] Path construction uses `pathlib.Path` or `os.path.join()`, not f-strings or string concatenation with `"/"`
- [ ] Tests use the same portable path APIs as production code

### Code Style
- [ ] PEP 8 style guide is followed
- [ ] Type hints are used consistently
- [ ] Code follows Python idioms and best practices
