
## Do one thing

A function should do one thing and do it well. You know you’re there when you can’t meaningfully extract another function from it without just renaming the original. If a function does “parse input, validate, call the API, and format the response,” it’s doing at least four things—split it. Single responsibility makes functions testable, readable, and reusable.

## One level of abstraction

Inside a function, stick to one level of abstraction. Don’t mix high-level “what we’re doing” with low-level “how we’re doing it” in the same block. Prefer a “stepdown” style: the function reads like a short narrative of steps, each step implemented by a call to a well-named helper (or the language’s standard API). That way the reader sees the algorithm at a glance and can drill into details only when needed.

## Keep them small

Short functions are easier to understand and change. Aim for a handful of lines; treat “under 15” as a good target and “under 25” as a normal maximum. If a function is pushing 50 lines, it’s almost certainly doing too much—split it. Length is a smell: when in doubt, extract.

## Few arguments

The ideal number of arguments is zero; one or two is fine. Three is already a lot. More than three usually means either the function is doing too much or the arguments belong in a struct/options object. Passing a single options/context object is clearer than a long parameter list. Boolean flags as arguments are a sign you should split the function: one function for “do X,” another for “do Y,” instead of `doThing(flag)`.

## No hidden side effects

The name and signature should tell the reader what the function does. If it “gets” something but also mutates state or triggers I/O, the name is misleading. Prefer **command–query separation**: a function either changes state (command) or returns a value (query), not both. So: no “getUserAndLogToAnalytics”—either return the user or do the logging in a separate call. Side effects (DB writes, sends, logs) should be obvious from the call site or the function name.

## Fail explicitly

Prefer returning errors or throwing exceptions to returning magic values (e.g. `null`, `-1`, `false`) that mean “something went wrong.” Callers should be able to handle failure without guessing. Use the language’s normal error mechanism; keep error messages clear and actionable. See error-handling and language-specific design rules for conventions.

## Don’t repeat yourself

Repeated blocks of logic belong in a shared function or a small helper. Duplication makes bugs and changes harder: you have to remember to update every copy. Extract common logic once, give it a clear name, and call it. That said, avoid speculative abstraction—only extract when you have real duplication or a clear, reusable idea.

## Name and doc only what’s needed

Function names should be verbs or verb phrases that describe the single thing the function does (see the naming rules). If the name and signature are clear, you often don’t need a docstring. Add a docstring when the function has non-obvious behavior, important preconditions, or side effects that aren’t obvious from the name. Don’t restate the signature in prose.
