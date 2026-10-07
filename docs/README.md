# MOV Machine

MOV-only FPGA processor in Amaranth, dependency-free Rust LLVM-IR compiler/assembler/emulator, combined Nix development environment, and independent formal specifications.

The compiler uses memory-alias equality and table selection from Stephen Dolan's [*mov is Turing-complete*](https://drwho.virtadpt.net/files/mov.pdf). Variable arithmetic, control flow and source-memory accesses lower to the existing four MOV instructions; the FPGA has no guest ALU. [Supported LLVM subset and ABI](compiler.md).

## Start

```sh
nix develop --impure
mkdir -p build
mov-compile examples/llvm/sum.ll build/sum.mov
mov-run build/sum.mov --max-steps 500000
mov-check
mov-formal
mov-build build/sum.mov
```

The example returns decoded `result: 42`; its architectural halt code identifies the result RAM region. The reference FPGA target is iCEBreaker; the build creates a real bitstream without automatically programming hardware.

## Documentation

* [Usage](usage.md): LLVM/C compilation, assembly, emulator/trace CLI, FPGA build/programming and verification commands.
* [Architecture](architecture.md): instruction encoding, memory map, reset/fault rules and RTL buses.
* [Compiler](compiler.md): supported LLVM IR, paper correspondence, Boolean arithmetic, logical memory and return ABI.
* [Formal verification](formal.md): RTL proof contract, finite TLA+/Alloy scopes and executable checks.
* [Environment](environment.md): provenance and combination of Rust, Amaranth, electronics and formal templates.
* [Verification results](verification.md): exercised checks and limits.

## Source layout

| Path | Responsibility |
|---|---|
| `hw/mov_machine/core.py` | Board-neutral MOV core |
| `hw/mov_machine/soc.py` | Synchronous ROM/RAM integration |
| `hw/mov_machine/build.py` | iCEBreaker wrapper and RTL/bitstream builds |
| `src/` | Rust LLVM parser/MOV compiler, assembler, executable format, emulator and CLI |
| `tests/` | Rust behavioral regressions |
| `formal/` | Independent RTL reference, induction/cover runner, TLA+ and Alloy models |
| `scripts/verify.py` | Actual CLI-to-RTL differential simulation |
| `examples/` | LLVM/C compiler inputs and handwritten pure-MOV programs |
| `docs/` | Project documentation |

Generated images, netlists, bitstreams, traces and solver artifacts live in ignored `build/`. Locks are retained as source.
