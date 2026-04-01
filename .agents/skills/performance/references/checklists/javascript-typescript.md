
# JavaScript/TypeScript-Specific Performance Standards

## Checklist
- [ ] collections are pre-allocated when size is known (e.g., array length hint where applicable)
- [ ] unnecessary allocations are avoided in hot paths (e.g., avoid creating arrays/objects in loops when avoidable)
- [ ] appropriate data structures are chosen for the use case (`Set`, `Map`, `Array`)
- [ ] array methods are used appropriately (`map`, `filter`, `reduce`, `forEach`)
- [ ] `for...of` loops are preferred over `for...in` for arrays
- [ ] object spread (`{...obj}`) is used instead of `Object.assign()` when possible
- [ ] template literals are used instead of string concatenation
- [ ] unnecessary array/object creation is avoided in loops
- [ ] `Set` and `Map` are used when appropriate (instead of objects/arrays)
