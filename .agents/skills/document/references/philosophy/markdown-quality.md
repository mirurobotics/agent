# Documentation style and quality guideline

Use this checklist when reviewing or writing documentation (markdown, MDX, docs sites). Prioritize: **Errors > Warnings > Suggestions**.

## Grammar & spelling

- [ ] No spelling errors or typos
- [ ] No grammar mistakes
- [ ] No awkward phrasing or unclear sentences
- [ ] Consistent tense (prefer present tense for docs)
- [ ] Correct punctuation

## Links & references

- [ ] Internal links point to existing files (verify with `ls` or glob)
- [ ] Anchor links (`#section-name`) are valid
- [ ] Snippet references (e.g. `<Snippet file="..." />`) point to existing files
- [ ] Image references resolve
- [ ] External links are well-formed

## Markdown & formatting

- [ ] Heading hierarchy is correct (do not skip levels, e.g. h1 → h3)
- [ ] List formatting is consistent (bullets vs numbers)
- [ ] Code blocks have language tags (e.g. ```python, ```bash)
- [ ] MDX component syntax is correct (when applicable)
- [ ] No unclosed tags or brackets

## Content quality

- [ ] No incomplete sentences or TODO markers left in
- [ ] No placeholder text (lorem ipsum, "TBD", "FIXME") unless intentional
- [ ] No outdated information that contradicts other docs
- [ ] Sufficient context so readers are not confused
- [ ] Steps are in order and none are missing

## Frontmatter (when applicable)

- [ ] Valid YAML syntax in frontmatter
- [ ] Required fields present (e.g. title, description)
- [ ] Description is meaningful (not a placeholder)

## Consistency

- [ ] Terminology matches the rest of the docs (e.g. check `snippets/definitions/` for canonical terms when present)
- [ ] UI element names match the actual product
- [ ] Command examples are accurate

## Review output

When presenting findings, use:

- **Errors** – must fix (e.g. broken links, unclosed blocks)
- **Warnings** – should fix (e.g. grammar, missing language tags)
- **Suggestions** – optional (e.g. simpler wording)

Always include line numbers and exact text. Verify links exist before reporting them broken. Focus on real issues; respect the repo’s conventions and intentional style.
