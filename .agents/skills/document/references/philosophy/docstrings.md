
# Docstrings

A docstring is the comment placed above a function, struct, method, etc. Callers read it when importing from another package or module.

**Philosophy:** The best documentation is good code. Write only the documentation that is necessary. Docstrings are good but not necessary—many functions, especially simple ones, don’t need one when the name and responsibility are clear.

Include a docstring when it provides information that cannot be easily guessed from the function’s name and signature, for example:
- Unintuitive side effects
- Important implementation details that affect usage
- Non-obvious parameter constraints or behaviors
- Performance characteristics that aren’t obvious

Private/internal functions have a much higher bar: usually they don’t need a docstring. If an internal function seems to need one, consider refactoring to make its purpose clearer first. Docstrings for private functions are still allowed when the behavior is particularly unintuitive.

**Code examples in documentation** should be simple, focused, complete and runnable when possible, and free of unnecessary complexity. Keep examples up to date with API changes.

**Checklist:** See the language-specific documentation rules in `documentation/checklists/` (Go, JavaScript/TypeScript, Python, Rust). Each has a Docstrings (or equivalent) section with the full checklist and conventions for that language.
