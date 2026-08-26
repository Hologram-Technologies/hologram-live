# Repository Agent Instructions

## Git workflow

- Keep the primary repository checkout on `main`. Before starting and before handing work back to the user, verify that it is on `main`, synchronized with `origin/main`, and has no unintended changes.
- Use a separate Git worktree and feature branch for tracked-file changes whenever practical. Treat the primary checkout as the stable place for updating and verifying `main`, not as the feature-development workspace.
- Create feature worktrees from an up-to-date `main`, and perform implementation, commits, pushes, tests, and pull-request updates inside the feature worktree.
- After a pull request merges, return to the primary checkout, run `git switch main` and `git pull --ff-only`, and confirm the final state with `git status --short --branch`.
- Do not leave the primary checkout on a feature branch at the end of a task, including when work is interrupted or blocked. Preserve any user-owned changes and never discard them merely to restore `main`.
