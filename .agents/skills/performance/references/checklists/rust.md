
# Rust-Specific Performance Standards

## Checklist
- [ ] collections are pre-allocated when size is known: `Vec::with_capacity(n)` and similar
- [ ] unnecessary allocations are avoided in hot paths
- [ ] appropriate data structures are chosen for the use case (`Vec`, `HashMap`, `HashSet`, `BTreeMap`, etc.)
- [ ] `&str` is preferred over `String` when ownership isn't needed
- [ ] `Cow<'_, str>` is used when flexibility between owned/borrowed is needed
- [ ] iterator chains are preferred over manual loops when readable
- [ ] `Box` is used only when necessary (avoid for small types)
