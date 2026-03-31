
# Go-Specific Performance Standards

## Checklist
- [ ] collections are pre-allocated when size is known: `make([]T, 0, capacity)`
- [ ] unnecessary allocations are avoided in hot paths
- [ ] appropriate data structures are chosen for the use case
- [ ] pointers are used only when sharing state or for large structs (avoid unnecessary heap allocations)
- [ ] reflection is only used if absolutely necessary
- [ ] `strings.Builder` is used for building strings instead of concatenation in loops
- [ ] `sync.Pool` is considered for frequently allocated/deallocated objects
