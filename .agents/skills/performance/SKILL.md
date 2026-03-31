---
name: performance
description: Evaluate and improve performance by identifying hot paths, reducing unnecessary allocations/work, and optimizing data flow and I/O behavior without changing required functionality. Use when addressing latency, throughput, CPU/memory overhead, startup cost, or scaling bottlenecks.
---

# Performance Workflow

## Inputs
- `scope`: files, module, package, service, or subsystem.
- `mode` (optional): `review-only` or `optimize` (default).
- `target_metric` (optional): latency, throughput, CPU, memory, startup, or mixed.

## References
Use selective loading:
- Always start with `references/philosophy/overview.md`.
- Load only active-language checklists:
- `references/checklists/go.md`
- `references/checklists/javascript-typescript.md`
- `references/checklists/python.md`
- `references/checklists/rust.md`
- In polyglot changes, load only the checklists for touched languages.

## Procedure
1. Identify measurable target metrics and likely hot paths.
2. Inspect algorithmic complexity, allocation patterns, I/O, and contention.
3. Propose optimizations with expected tradeoffs.
4. In `optimize` mode, implement focused changes with minimal behavioral risk.
5. Validate with benchmarks/profiling/tests where available.
6. Summarize gains, tradeoffs, and follow-up opportunities.

## Performance Focus Areas
- Algorithmic complexity and unnecessary work.
- Allocation frequency and object lifetime.
- I/O batching/caching and round-trip reduction.
- Concurrency/parallelism overhead and lock contention.
- Data structure choices for access patterns.

## Output Contract
1. Performance findings (bottleneck + evidence).
2. Optimization summary (when `mode=optimize`).
3. Measurement/validation results and residual tradeoffs.
