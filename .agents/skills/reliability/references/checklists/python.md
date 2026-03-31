
# Python Reliability Standards

## Resource management and cleanup
- [ ] Files and file-like objects are opened with `with` statements so they are closed even on exception
- [ ] Database connections and connection pools use context managers or explicit close in finally
- [ ] Locks, semaphores, and other synchronization primitives are released in finally or via context managers
- [ ] Long-lived resources (servers, thread pools) have an explicit shutdown path (atexit, signal handlers, or lifecycle hooks)
- [ ] Cleanup runs even when errors occur; use try/finally or context managers so release is not skipped

## Async and concurrency
- [ ] Async context managers (`async with`) are used for async resources (connections, locks)
- [ ] asyncio tasks are cancelled or awaited so they don’t outlive their scope; task groups or explicit cancellation where appropriate
- [ ] Locks and resources in async code are released in finally or via async context managers

## Error handling and recovery
- [ ] Exceptions are handled or re-raised; they are not silently swallowed (no bare except that ignores)
- [ ] Cleanup (close, release) runs in finally or context manager __exit__ even when exceptions occur
- [ ] Retries use backoff and bounded attempts or timeouts where appropriate
- [ ] Exception chaining is used when re-raising so the cause is preserved for debugging

## Timeouts and bounds
- [ ] I/O and external calls use timeouts (e.g. socket timeout, asyncio.wait_for) where appropriate
- [ ] No unbounded blocking on network or external calls; timeouts allow the process to shut down or retry sensibly
