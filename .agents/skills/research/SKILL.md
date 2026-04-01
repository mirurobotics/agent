---
name: research
description: Create and update research documents in research/ for technical investigations, vendor evaluations, architecture decisions, and product research. Use when asked to start research, write up findings, or update an existing research doc.
---

# Research Workflow

Research documents live in `research/` at the repo root.

## Inputs
- `mode`: `new` (default) or `update`.
- `topic`: subject of the research.
- `slug` (optional): filename slug. Derived from topic if not provided.
- `path` (optional): path to existing document. Required for `update` mode if ambiguous.

## References
- `references/sections.md`: suggested sections organized by research type. Load when scaffolding a new document.
- Existing docs in `research/`: scan for naming and formatting conventions before creating new files.

## Procedure

### Mode: new

1. Derive slug from topic if not provided. Format: lowercase, hyphenated, concise.
2. Determine if the research needs a single file or a directory (directory when multiple sub-documents are expected — e.g., separate architecture doc, conversation log, or sub-topic deep-dives).
3. Create file at `research/YYYYMMDD-<slug>.md` or directory at `research/YYYYMMDD-<slug>/` with a `research.md` entry point.
4. Write the required header block (see Header below).
5. Load `references/sections.md` and select sections appropriate to the topic. Include Problem/Context and at least one substantive section. Do not include empty placeholder sections.
6. Write initial content. Prefer substance over structure — a short document with real findings beats a long skeleton of empty headings.

### Mode: update

1. Locate the target document. If `path` is not provided, search `research/` by topic/slug.
2. Update the **Status** field if the research has progressed (see Status Lifecycle).
3. Add or revise content sections as needed. Preserve the author's existing structure — do not reorganize unless asked.
4. If new sub-documents are needed and the research is currently a single file, offer to convert to a directory structure.

## Header

Every research document must start with an H1 title and a metadata block. Use bold key-value pairs on separate lines directly under the title:

```
# Title

**Date:** YYYY-MM-DD
**Status:** Research | Draft | Proposal | Decision | Superseded
**Author:** name or team
```

Additional metadata fields are optional and encouraged when relevant: **Scope**, **Context**, **Purpose**. Keep the block short.

## Status Lifecycle

Research → Draft → Proposal → Decision → Superseded

Not every document progresses through all stages. A vendor evaluation may go Research → Decision. A brainstorm may stay at Research indefinitely. Use the status that honestly reflects the document's state.

## Naming

- Single file: `research/YYYYMMDD-<slug>.md`
- Directory: `research/YYYYMMDD-<slug>/research.md` (entry point), with companion files as needed.
- Slug: lowercase, hyphenated, descriptive. Match the precision level of existing docs (e.g., `soc2-compliance-vendor-research`, not `soc2`).

## Output Contract

| Mode | Output |
|------|--------|
| new | File path created. Title, status, and initial sections written. |
| update | File path updated. Changes summarized (status change, sections added/revised). |
