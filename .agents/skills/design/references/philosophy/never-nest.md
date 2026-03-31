
## Principle

Deep nesting (many levels of `if` inside `if`, or inside loops) makes code harder to read and reason about. The important logic ends up buried and indented, and each extra level adds cognitive load. Prefer **early exits**: handle invalid or edge cases first, then keep the main path at a shallow, consistent indentation. This applies in both loops (use `continue` or `break`) and functions (use `return` or `return error`).

This document uses Go for examples. However, the same ideas apply in other languages (e.g. early `return` in JavaScript/TypeScript, `continue`/`break` in loops, guard clauses in Rust).

---

## In loops

Avoid nesting multiple `if` blocks inside a loop. Instead, check disqualifying conditions first and `continue` (or `break`) so the “happy path” stays at top level inside the loop.

**Problematic deep nesting (should be refactored):**

```go
// ❌ BAD - Deep nesting
for item := range items {
    if item != nil {
        if item.IsValid() {
            if item.CanProcess() {
                // actual logic buried deep
                process(item)
            }
        }
    }
}
```

**Refactored with early exits:**

```go
// ✅ GOOD - Early returns reduce nesting (in loops)
for item := range items {
    if item == nil {
        continue
    }
    if !item.IsValid() {
        continue
    }
    if !item.CanProcess() {
        continue
    }
    // actual logic at top level
    process(item)
}
```

Each condition is a guard: “if this item doesn’t qualify, skip it.” The code that actually does the work stays at one indentation level and is easy to see.

---

## In functions

Same idea for functions: handle errors and edge cases first, then `return` so the main logic isn’t wrapped in many `else` branches. This is sometimes called “guard clauses” or “fail fast.”

**Problematic deep nesting in a function:**

```go
// ❌ BAD - Deep nesting in function
func processItem(item *Item) error {
    if item != nil {
        if item.IsValid() {
            if item.CanProcess() {
                // actual logic buried deep
                return item.Process()
            } else {
                return fmt.Errorf("item cannot be processed")
            }
        } else {
            return fmt.Errorf("item is invalid")
        }
    } else {
        return fmt.Errorf("item is nil")
    }
}
```

**Refactored with early returns:**

```go
// ✅ GOOD - Early returns from function
func processItem(item *Item) error {
    if item == nil {
        return fmt.Errorf("item is nil")
    }
    if !item.IsValid() {
        return fmt.Errorf("item is invalid")
    }
    if !item.CanProcess() {
        return fmt.Errorf("item cannot be processed")
    }
    // actual logic at top level
    return item.Process()
}
```

Readers see the preconditions and their error messages in order, then the single success path at the end. No need to mentally unwind nested `else` branches.

---

## When nesting is acceptable

Shallow, two-branch structure is fine and often clearer than forcing an early return:

```go
if condition {
    // main logic
} else {
    // alternative logic
}
```

One level of nesting (e.g. one `if` inside a loop, or one `if`/`else` block) is usually acceptable. The rule of thumb is to **avoid going beyond that**: if you find yourself three or more levels deep, refactor with early exits so the main path stays at a consistent, shallow level.
