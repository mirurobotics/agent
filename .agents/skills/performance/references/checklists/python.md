
# Python-Specific Performance Standards

## Checklist
- [ ] collections are pre-allocated when size is known (e.g., list with known length, reserve capacity where applicable)
- [ ] unnecessary allocations are avoided in hot paths (e.g., generators instead of building large lists)
- [ ] appropriate data structures are chosen for the use case (list, dict, set, deque, etc.)
- [ ] list comprehensions are preferred over `map()`/`filter()` for readability
- [ ] `collections.deque` is used for queue operations (O(1) append/pop)
- [ ] `collections.defaultdict` or `dict.get()` is used to avoid KeyError
- [ ] `f-strings` are used for string formatting (Python 3.6+)
- [ ] unnecessary list creation is avoided (use generators when possible)
- [ ] `itertools` is used for efficient iteration patterns
