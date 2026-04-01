
# Rust Reliability Standards

## Resource management and cleanup
- [ ] Types that own resources implement `Drop` so cleanup runs when values go out of scope
- [ ] RAII is used: acquire in constructor or entry, release in Drop; no manual “forget to close” paths
- [ ] File handles, network connections, and other system resources are closed via Drop or explicit close in Drop
- [ ] Locks and guards are held for the minimum time necessary; no long-lived holding across await or I/O when avoidable
- [ ] Async resources (connections, streams) are dropped or explicitly closed when tasks are cancelled or dropped

## Concurrency and lifecycle
- [ ] Spawned tasks (e.g. tokio::spawn) have clear ownership; cancellation tokens or channels are used to stop them when the parent is done
- [ ] No resource or handle is shared across threads without proper synchronization (Send/Sync, Arc, etc.)
- [ ] Shutdown and cancellation are propagated so in-flight work can finish or release resources

## Error handling and recovery
- [ ] Errors are propagated with `?` or handled; they are not ignored with `let _ =` in production paths
- [ ] Cleanup runs when errors occur (Drop runs on panic and normal exit; ensure no early return skips release)
- [ ] Retries use backoff and bounded attempts or timeouts where appropriate

## Timeouts and bounds
- [ ] I/O and external calls use timeouts (e.g. tokio::time::timeout, request timeouts) where appropriate
- [ ] No unbounded blocking on network or external calls; timeouts or cancellation allow graceful shutdown

## Dependencies
- [ ] Dependency versions are pinned to exact semver (e.g., `"2.0.18"` not `"2"`)
- [ ] Workspace dependencies use `{ workspace = true }` in member crates
