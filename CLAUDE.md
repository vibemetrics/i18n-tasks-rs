@AGENTS.md

## Our fork

This is `vibemetrics/i18n-tasks-rs`, Bondata's fork of `rwojsznis/i18n-tasks-rs`. `origin` is the fork and `upstream` is the original. Pull requests go to the fork; an upstream contribution is a separate PR, cut from `upstream/main`.

**`main` changes by merges only.** It is never rebased and never force-pushed.

**Review feedback.** Copilot's findings are advice. Fix a real defect or a real risk. Answer a nitpick in one line, saying why the code stays, and resolve the thread.

**Merging.** Pull requests here get no human review. A PR merges when Copilot's review recommends it, and the agent merges it itself: squash, with the PR's title and body. A sync from `upstream/main` is the exception: it lands as a merge commit, so upstream's history stays in ours and the next sync does not conflict again.

**Releasing.** A tag `vX.Y.Z-bondata.N` on `main` publishes a GitHub release with the Linux x86_64 tarball and `SHA256SUMS.txt`. `X.Y.Z` is the upstream version the fork is built on, and `N` counts our releases on it. A tag is new for each release and never moved:

```bash
git tag v0.1.3-bondata.1 origin/main && git push origin v0.1.3-bondata.1
```
