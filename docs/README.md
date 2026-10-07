# MOV Machine

MOV-only FPGA processor in Amaranth, dependency-free Rust assembler/emulator, combined Nix development environment, and independent formal specifications.

**Compatibility boundary:** the upstream [Movsucator compiler](https://github.com/Hussdev0/movsucator) does not currently define complete executable output. This project implements its four concrete pure-MOV assembly forms with a local encoding; it does not claim arbitrary compiler-output compatibility. [Evidence and requirements](compiler.md).

## Start

```sh
nix develop --impure
mkdir -p build
mov-assemble examples/copy.s build/copy.mov
mov-run build/copy.mov --max-steps 1000
mov-check
mov-formal
mov-build build/copy.mov
```

The example halts with code 42. The reference FPGA target is iCEBreaker; the build creates a real bitstream without automatically programming hardware.

## Documentation

* [Usage](usage.md): environment, assembly, emulator/trace CLI, FPGA build/programming and verification commands.
* [Architecture](architecture.md): instruction encoding, memory map, reset/fault rules and RTL buses.
* [Compiler compatibility](compiler.md): pinned upstream evidence, accepted grammar and missing backend prerequisites.
* [Formal verification](formal.md): RTL proof contract, finite TLA+/Alloy scopes and executable checks.
* [Environment](environment.md): provenance and combination of Rust, Amaranth, electronics and formal templates.
* [Verification results](verification.md): exercised checks and limits.

## Source layout

| Path | Responsibility |
|---|---|
| `hw/mov_machine/core.py` | Board-neutral MOV core |
| `hw/mov_machine/soc.py` | Synchronous ROM/RAM integration |
| `hw/mov_machine/build.py` | iCEBreaker wrapper and RTL/bitstream builds |
| `src/` | Rust assembler, image format, architectural emulator and CLI |
| `tests/` | Rust behavioral regressions |
| `formal/` | Independent RTL reference, induction/cover runner, TLA+ and Alloy models |
| `scripts/verify.py` | Actual CLI-to-RTL differential simulation |
| `examples/` | Executable handwritten pure-MOV programs |
| `docs/` | Project documentation |

Generated images, netlists, bitstreams, traces and solver artifacts live in ignored `build/`. Locks are retained as source.
