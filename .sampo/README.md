# Changesets

Reapr's versions and changelog are managed by [Sampo](https://github.com/bruits/sampo/blob/main/crates/sampo/README.md).
A pull request that changes what users get adds a changeset to `changesets/`:

```sh
sampo add -p cargo/rpr -b minor -t Added -m 'One line, written for users.'
```

or run `sampo add` and answer the prompts. The file it writes looks like this:

```md
---
cargo/rpr: minor (Added)
---

One line, written for users.
```

Always name `cargo/rpr`: its changelog, [crates/rpr/CHANGELOG.md](../crates/rpr/CHANGELOG.md), is Reapr's
changelog, and the three crates share one version. Tags: `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`,
`Security`. See [CONTRIBUTING.md](../CONTRIBUTING.md#changesets) for when one is needed and how releases happen.
