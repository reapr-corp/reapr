---
status: proposed
---

# Out-of-process adapters speak JSON over stdio (protocol v1)

Some engines are best driven from their own language: PIT and Infection expose their internals to Java and
PHP, not to Rust. For those, the adapter is a separate executable, `rpr-adapter-<engine>` on `PATH`, written
in the engine's language, and `rpr` talks to it through one in-tree adapter that implements
`MutationEngineAdapter` (ADR 0001) by speaking this protocol. To be built with the third engine, not before:
two engines are not enough to know the protocol is right.

## Protocol v1

- **Transport:** the adapter's stdin and stdout, one JSON object per line (JSON Lines), UTF-8. stderr is the
  engine's output and goes to the user's stderr unchanged, as for in-tree adapters. One request at a time;
  `rpr` waits for the response (no polling).
- **Handshake:** `rpr` sends `{"type":"hello","protocol":1}`; the adapter answers
  `{"type":"hello","protocol":1,"engine":"pit","engine_version":"1.17.0","capabilities":["detect","run"]}`.
  A protocol `rpr` does not speak, or an engine id that differs from the executable's name, is an error.
- **Requests:** `{"id":1,"method":"<method>","params":{...}}` with methods `detect` (`project_dir`),
  `list` (run context and scope), `run` (run context and shard) and `diagnose`. Params and results are the
  core types serialized as in `report.json` (`RunContext`, `MutationScope`, `Shard`, `Detection`, `Mutant`,
  `RunOutcome`). A method missing from `capabilities` is not sent; `rpr` maps it to
  `AdapterError::Unsupported`.
- **Responses:** `{"id":1,"result":{...}}` or `{"id":1,"error":{"kind":"...","message":"..."}}`, where `kind`
  is one of `AdapterError`'s variants (`unsupported`, `config_missing`, `io`, `process`, `parse`) so errors
  keep their type across the process boundary.
- **No streaming in v1:** a run returns one `RunOutcome` when it ends. Progress is the engine's own stderr.
- **Lifecycle:** one adapter process per `rpr` invocation; `rpr` closes stdin when done and the adapter
  exits. A crash or a malformed line is `AdapterError::Process` naming the executable.
- **Discovery:** an engine id with no in-tree adapter is looked up as `rpr-adapter-<engine>` on `PATH`, like
  `cargo` subcommands; `--engine <id>` selects it.

## Considered Options

- **Dynamic libraries / plugins (C ABI, WASM).** Rejected: the adapter must run inside the engine's own
  runtime (a JVM, PHP), which a shared library cannot host.
- **gRPC or HTTP.** Rejected: a port, a server and a dependency for what is one request at a time between a
  parent and its child.
