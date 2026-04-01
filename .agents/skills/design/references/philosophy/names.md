
## Intent

Names should reveal intent. A name ought to answer: *what is this?* and *why is it here?* If you need a comment to explain what a variable or function is for, the name is likely too vague. Prefer `elapsedSecs` over `d`, `userCount` over `n`, and `isValid` over `flag`.

## Avoid disinformation

- Don’t use names that look or sound almost the same but mean different things (e.g. `controller` vs `controllerManager` only by a suffix).
- Don’t use names that imply a type or meaning they don’t have (e.g. a list named `accountList` that isn’t a list).
- Avoid encodings (no Hungarian notation, no prefix/suffix that just restates the type). Let the type system and IDE show types.

## Make meaningful distinctions

- Don’t distinguish names only by number or noise: `data` / `data2`, `info` / `theInfo` force the reader to remember the difference. Use names that state the real difference (e.g. `sourceAccount` vs `targetAccount`).
- Don’t use the same word for different concepts. If “get” means “return from memory” in one place, don’t use “get” for “fetch from network” elsewhere—pick another verb (e.g. `fetch`, `load`) and use it consistently.

## One word per concept

Pick one verb per idea and stick to it across the codebase: e.g. use `get` everywhere for “return without side effects,” or `fetch` everywhere for “I/O.” Don’t mix `get` / `fetch` / `retrieve` for the same idea. Same for other concepts (e.g. one term for “representation of a user in the API,” not `user` in one module and `account` in another for the same thing).

## Pronounceable and searchable

- Use names you can say out loud; avoid arbitrary abbreviations that only you understand.
- Prefer names that are easy to search for. Single-letter names are only for very short-lived, obvious scope (e.g. loop index `i`, or `e` for a single catch block). Longer-lived or shared names should be searchable.

## Booleans and predicates

Booleans and predicate functions should read as yes/no questions: `isValid`, `hasPermission`, `canEdit`, `shouldRetry`. Avoid a name that sounds like a noun and only happens to be a boolean (e.g. `valid` is weaker than `isValid`).

## Classes and types: nouns; functions: verbs

- Classes and types are things: nouns or noun phrases (e.g. `User`, `Order`, `PaymentProcessor`).
- Functions and methods do something: verbs or verb phrases (e.g. `save`, `parseRequest`, `getUserById`). Accessors can be `getX`/`isX`/`hasX`; mutators can be `setX` or verb phrases like `enableNotifications`.

## Context, not redundancy

Add context when the name would otherwise be ambiguous, but don’t repeat what scope or type already gives. `User.name` doesn’t need to be `userName` inside the `User` class; `name` is enough. In a small function, `count` may be clearer than `loopCounter` if there’s only one loop. In a large scope, a longer name like `activeUserCount` may be necessary.

## Don’t be cute

Avoid jokes, puns, or culture-specific slang. Names are for clarity and long-term maintenance; the next reader may not share the same context. Prefer boring and obvious over clever and obscure.
