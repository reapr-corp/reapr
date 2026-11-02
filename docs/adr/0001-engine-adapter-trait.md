---
status: accepted
---

# One async trait per engine, dispatched through an enum

Every engine is reached through `MutationEngineAdapter` in `reapr-core`: native `async fn` in trait, with
futures declared `+ Send`, deliberately not dyn-compatible. The CLI dispatches over the adapters built into
it with `enum Engine { Stryker(..), CargoMutants(..) }`, one `match` arm per adapter and method. We chose this
over `Box<dyn Adapter>` (which needs `async-trait` boxing or hand-written boxed futures on every call) because
the set of in-tree adapters is closed and known at compile time: the enum costs one arm per method per
engine, gives exhaustive matches when an engine is added, and keeps the trait free of object-safety
constraints. The trait itself is owned by the core and changes only with the owner's approval
(`docs/design/phase-1.md`).

## Considered Options

- **`dyn` trait objects.** Rejected: open extension is not needed in-tree, and it would force boxing on the
  hot path of every call and `Send` bounds that are harder to read.
- **One binary per engine.** Rejected: one portable `rpr` is the product; the user should not pick a binary.

## Consequences

- Adding an in-tree engine touches `crates/rpr/src/main.rs`: a variant, an entry in `Engine::all()` (the
  detection order), and an arm in each dispatch method.
- Engines that cannot live in-tree (adapters written in the engine's own language) do not implement this
  trait directly; they are reached through one in-tree adapter that speaks a protocol (ADR 0003).
