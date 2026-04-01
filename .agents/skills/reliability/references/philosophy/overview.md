
# Reliability

**Reliability is a core promise.** At Miru we build systems that behave predictably under load and failure. Leaks, dangling resources, and unhandled errors lead to outages and data loss. Every change is reviewed with reliability in mind.

## Philosophy

- **Resource discipline.** Every resource acquired must be released. Use the language’s canonical mechanism (defer, context managers, RAII, cleanup callbacks) and use it consistently. No “hope the GC cleans it up” for files, connections, or locks.
- **Fail safely.** Errors must be handled or propagated; they must not be silently swallowed. Cleanup must run even when errors occur—cleanup paths are not optional. Default to failing closed (deny, release, roll back) when something goes wrong.
- **Bounded operations.** Avoid unbounded waits and unbounded retries. Set timeouts on I/O and external calls. Use context cancellation (Go), AbortController (JS), or equivalent so in-flight work can be stopped and resources released.
- **Observable and recoverable.** Log failures and resource lifecycle events where they help debugging. Design so that restarts and retries are safe (idempotency where it matters, no leaked state across restarts).

Checklist items are in `reliability/checklists/` (Go, JavaScript/TypeScript, Python, Rust). Use the checklist for your language when reviewing or writing code.
