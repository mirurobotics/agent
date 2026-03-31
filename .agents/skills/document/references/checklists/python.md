
# Python-Specific Documentation Standards

This file contains Python-specific documentation recommendations.

## Module Documentation
Every module should have a docstring at the top of the file (after shebang and encoding, if present) explaining the module's purpose. Module docstrings are triple-quoted strings.

Module documentation should:
- Explain the module's purpose and responsibility
- Provide usage examples if the module has a primary API
- Reference external resources when relevant

### Checklist
- [ ] Public modules have docstrings explaining their purpose
- [ ] Module docstrings include usage examples if applicable
- [ ] Module docstrings reference external resources when relevant

## Docstrings
Python uses docstrings (triple-quoted strings) for documentation. Docstrings follow PEP 257 conventions and can use various styles (Google, NumPy, Sphinx).

**Conventions:**
- Docstrings use triple quotes (`"""` or `'''`)
- First line is a brief summary (one line)
- Blank line separates summary from detailed description
- Use Google-style or NumPy-style for structured documentation
- Include `Args:`, `Returns:`, `Raises:` sections when appropriate

**Example (Google style):**
```python
def process_item(item: Item) -> Result:
    """Process a single item and return the result.
    
    The item must be in a valid state before processing. This function is
    safe to call concurrently.
    
    Args:
        item: The item to process. Must be in a valid state.
    
    Returns:
        A Result object containing the processing outcome.
    
    Raises:
        ValueError: If the item is in an invalid state.
        ProcessingError: If processing fails.
    """
    # ...
```

**Example (NumPy style):**
```python
def process_item(item: Item) -> Result:
    """Process a single item and return the result.
    
    Parameters
    ----------
    item : Item
        The item to process. Must be in a valid state.
    
    Returns
    -------
    Result
        A Result object containing the processing outcome.
    
    Raises
    ------
    ValueError
        If the item is in an invalid state.
    ProcessingError
        If processing fails.
    """
    # ...
```

### Checklist
- [ ] docstrings are omitted if the code is sufficiently self-explanatory; docstrings are included if the code has unguessable or unintuitive side-effects
- [ ] docstrings are only used for private components if particularly unintuitive
- [ ] docstrings give more information than can be easily guessed by the function's name
- [ ] Public functions, classes, and methods have docstrings
- [ ] Docstrings use triple quotes
- [ ] First line is a brief summary
- [ ] Detailed description follows (if needed)
- [ ] `Args:`/`Parameters:` section documents all parameters
- [ ] `Returns:` section documents return value
- [ ] `Raises:` section documents exceptions
- [ ] Docstring style is consistent within the project (Google, NumPy, or Sphinx)

## Type Hints in Documentation
Type hints in function signatures serve as documentation. Docstrings should complement, not duplicate, type information.

### Checklist
- [ ] Type hints are present in function signatures
- [ ] Docstrings add context beyond what type hints provide
- [ ] Complex types are explained in docstrings even if type hints exist

## Inline Documentation
Python also supports inline comments and type comments (for older Python versions).

### Checklist
- [ ] inline comments do not explain what the code is doing (the code should do this)
- [ ] inline comments are used to explain unintuitive sections or algorithms (the "why" not the "what")

## Examples
Python docstrings can include usage examples. These examples:
- Are placed in docstrings under an `Examples:` section
- Can be tested with `doctest` module
- Should be runnable and demonstrate common usage

**Example:**
```python
def calculate_total(items: list[float]) -> float:
    """Calculate the total of a list of items.
    
    Examples:
        >>> calculate_total([1.0, 2.5, 3.0])
        6.5
        >>> calculate_total([])
        0.0
    """
    return sum(items)
```

### Checklist
- [ ] examples are clear and demonstrate the documented feature
- [ ] examples don't include unnecessary complexity
- [ ] examples are kept up to date with API changes
- [ ] Examples are included in docstrings for complex or non-obvious APIs
- [ ] Examples are runnable and demonstrate common usage patterns
- [ ] Examples can be tested with `doctest` when appropriate
