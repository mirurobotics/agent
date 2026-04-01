
# JavaScript/TypeScript Reliability Standards

## Resource management and cleanup
- [ ] Event listeners are removed when no longer needed (removeEventListener, or cleanup in useEffect)
- [ ] Subscriptions, timers, and intervals are cancelled or cleared when components unmount or when no longer needed (clearInterval, clearTimeout, unsubscribe)
- [ ] Promise chains and async code use try/finally or proper error handling so cleanup runs even on failure
- [ ] AbortController (or equivalent) is used to cancel in-flight requests when the component or flow is torn down
- [ ] Open handles (files, sockets, DB connections in Node) are closed in finally blocks or cleanup callbacks

## Async and concurrency
- [ ] Async operations are not left dangling; rejections are caught or propagated, not unhandled
- [ ] Race conditions are avoided (e.g. don’t update state after unmount; use refs or cancellation)
- [ ] Long-running or repeated work is tied to component/flow lifecycle and stopped on unmount or navigation

## Error handling and recovery
- [ ] Promise rejections and async errors are handled (catch, try/await) so they don’t leave resources dangling
- [ ] Errors are logged or surfaced; they are not silently swallowed
- [ ] Retries use backoff and bounded attempts or timeouts where appropriate

## Timeouts and bounds
- [ ] Network and I/O calls use timeouts or AbortSignal so they don’t block indefinitely
- [ ] Polling or repeated work has a clear stop condition (unmount, cancel, or max iterations)
