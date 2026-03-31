
# Go-Specific Documentation Standards

This file contains Go-specific documentation recommendations.

## Package Documentation
Every package should maintain a `doc.go` file which gives a high-level overview of the package's responsibility. `doc.go` files shouldn't go into much detail of individual functions and data structures--modern code editors make it easy to explore all of the functions in a package.

However, it is conventional to include the most common usage of the package. If there are a clear set of one to three functions which are the primary usage of the package, then they should be included under a 'Usage:' header. Otherwise, omitting the package's common usage is perfectly fine.

Finally, `doc.go` files are an ideal place for resources used to write the code. This is most salient for packages which interact with third party services. An 's3' package would include references to s3 documentation. An API key package would include references to writing secure code for API keys. Include references under a 'References:' header.

### Checklist
- [ ] Every package has a `doc.go` file associated with it
- [ ] `doc.go` is descriptive but concise, giving users an idea of what the package is used for and if the functionality they need can be found inside
- [ ] `doc.go` file includes common usage examples if applicable
- [ ] `doc.go` file includes relevant resources used to write the package

## Inline comments
Inline comments explain unintuitive parts of the code, not what the code is doing.

### Checklist
- [ ] inline comments do not explain what the code is doing (the code should do this)
- [ ] inline comments are used to explain unintuitive sections or algorithms
- [ ] complex algorithms or business logic have brief comments explaining the "why" not the "what"

## File Headers
Go discourages file header documentation. Whatever you'd place in a file header nearly always belongs in the `doc.go` file for the package itself. If you find that you *really* need a file header, consider that the file may need to be split into its own package.

### Checklist
- [ ] File headers are not used

## Docstrings
Go docstrings are comments placed directly above exported functions, types, variables, and constants. They follow specific conventions:

- Docstrings begin with the name of the function/type they document (Go convention)
- They use complete sentences
- The first sentence should be a summary (used by `go doc`)
- Multi-paragraph docstrings are separated by blank comment lines

**Example:**
```go
// ProcessItem processes a single item and returns an error if processing fails.
// The item must be in a valid state before processing. This function is
// safe to call concurrently.
func ProcessItem(item *Item) error {
    // ...
}
```

### Checklist
- [ ] docstrings are omitted if the code is sufficiently self-explanatory; docstrings are included if the code has unguessable or unintuitive side-effects
- [ ] docstrings are only used for private components if particularly unintuitive
- [ ] docstrings give more information than can be easily guessed by the function's name
- [ ] docstrings begin with the name of the function/type they document (Go convention)
- [ ] exported functions, types, and variables have docstrings
- [ ] docstrings use complete sentences
- [ ] first sentence is a summary suitable for `go doc` output

## Examples
Go supports example functions that are automatically included in documentation. Example functions:
- Are named `Example`, `ExampleType`, or `ExampleType_Method`
- Are placed in `*_test.go` files
- Use `// Output:` comments to show expected output

### Checklist
- [ ] examples are clear and demonstrate the documented feature
- [ ] examples don't include unnecessary complexity
- [ ] examples are kept up to date with API changes
- [ ] example functions are used for complex or non-obvious APIs
- [ ] example functions demonstrate common usage patterns
