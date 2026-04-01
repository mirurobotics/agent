---
name: install-skills
description: Install skills from skills.sh, rename to short names, and track originals for updates.
disable-model-invocation: true
---

# Install Skills

Install skills from [skills.sh](https://skills.sh) into `.agents/skills/`, rename to short names, and track origins for future updates.

## Instructions

### Installing new skills

1. Run the install command:
   ```bash
   npx skills add <owner/repo> -y
   ```
2. After install, move new skills from the downloaded `.agents/skills/` to `.agents/skills/`:
   ```bash
   cp -R .agents/skills/* .agents/skills/ && rm -rf .agents/
   ```
3. For each new skill, pick the **shortest clear name** that isn't already taken. Rename the folder and update the `name` field in SKILL.md to match.
4. Add source tracking metadata to the skill's SKILL.md frontmatter:
   ```yaml
   metadata:
     original-name: <original-folder-name>
     source: <owner/repo>
   ```
5. **CRITICAL: If the skill contains an AGENTS.md, delete it.** Cursor auto-applies any AGENTS.md in subdirectories, wasting thousands of tokens on every prompt. The SKILL.md already contains the summary the agent needs.
6. **Update the registry table above** in this file with the new skill's short name, original name, and source repo.
7. List the installed skills and confirm they work.

### Updating existing skills

1. Check the registry table above to find the source repo and original name.
2. Run `npx skills add <source-repo> -y` to re-download.
3. The new version will land in `.agents/skills/<original-name>/`.
4. Copy it over the existing short-named folder in `.agents/skills/`:
   ```bash
   cp -R .agents/skills/<original-name>/* .agents/skills/<short-name>/ && rm -rf .agents/
   ```
5. Re-apply the `name` field rename and source metadata in the SKILL.md frontmatter.
6. If the updated skill has an AGENTS.md, delete it (see step 5 in "Installing new skills").

### Removing skills

1. Delete the folder from `.agents/skills/`.
2. Remove the entry from the registry table above.

### Naming conventions

- Use the shortest name that's still clear (e.g., `react-perf` not `vercel-react-best-practices`).
- Only make names longer if there's a collision.
- Lowercase with hyphens only.