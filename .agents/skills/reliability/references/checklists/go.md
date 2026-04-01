
# Go Reliability Standards

## Resource management and cleanup
- [ ] `defer` is used for cleanup (close, unlock, release) so it runs even when errors occur
- [ ] Resources acquired are always released; no leaks (files, connections, goroutines, locks)
- [ ] Cleanup order is correct (LIFO via defer stack); dependencies are cleaned in reverse order of acquisition
- [ ] `defer` is used immediately after acquiring a resource, not only on the happy path
- [ ] Long-lived resources (listeners, pools) have a clear shutdown path and are closed on context cancellation or process exit

## Concurrency and lifecycle
- [ ] Goroutines have clear ownership and termination; they exit when context is cancelled or work is done
- [ ] Context cancellation is propagated to child goroutines and used to stop work and release resources
- [ ] `sync.WaitGroup` or similar is used when the main flow must wait for goroutines to finish
- [ ] Channels are closed by the sender; receivers handle closed channels and exit

## Error handling and recovery
- [ ] Errors are not silently ignored; they are handled, logged, or returned
- [ ] Cleanup runs even when errors occur (defer ensures this; avoid early returns that skip cleanup)
- [ ] Retries use backoff and a bounded number of attempts or a timeout where appropriate

## Timeouts and bounds
- [ ] External I/O and RPCs use context with timeout or deadline where appropriate
- [ ] No unbounded blocking on external calls; timeouts or cancellation are used so the process can shut down cleanly
