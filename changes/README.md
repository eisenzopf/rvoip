# Change fragments

Every pull request that changes what a user of rvoip sees adds one file here:
`changes/<pr-number>.md`, or `changes/<pr-number>-<slug>.md` when one PR has
several entries.

```markdown
---
section: Fixed
breaking: false
---
TLS flows apply back-pressure instead of dropping responses when the
writer falls behind.
```

- `section` is one of `Breaking changes`, `Added`, `Changed`, `Fixed`,
  `Performance`, `Security`, `Deprecated`, `Removed`, `Documentation`.
- `breaking: true` (implied by `section: Breaking changes`) lists the entry in
  the release notes' breaking-changes summary; it must say how to migrate.
- The body is one or a few Markdown paragraphs, written for a user, without
  headings. The PR number is appended for you.

The PR Gate validates every fragment (`python3 scripts/release/notes.py check`).
The **Prepare release PR** workflow appends them to `## Unreleased` in
`CHANGELOG.md`, renders them into `docs/release/RELEASE_NOTES_NEXT.md`, and
deletes them in the release PR. Hand-written changelog prose (for example the
`### Highlights`) stays as it is.
