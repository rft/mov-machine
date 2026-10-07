# MOV Machine

A MOV-only FPGA processor in **Amaranth**, with a dependency-free **Rust LLVM-IR compiler, assembler and emulator**, a combined **Nix development shell**, and independent machine formal specifications.

The guest ISA has four MOV forms and no arithmetic opcodes or MMIO ALU. The compiler builds arithmetic and control flow from memory aliasing and table selection, following Stephen Dolan's [*mov is Turing-complete*](https://drwho.virtadpt.net/files/mov.pdf).

**[Project overview and diagrams](docs/README.md)** explain how the compiler, emulator, hardware, verification and development tools fit together.

## Quick start

```sh
nix develop --impure
mkdir -p build
mov-compile examples/llvm/sum.ll build/sum.mov
mov-run build/sum.mov --max-steps 500000
```

The example returns decoded `result: 42`. Its architectural `halt_code` is the address of the encoded result in RAM, not the returned integer. The compiler accepts a [defined integer/pointer LLVM subset](docs/compiler.md#source-contract), not arbitrary LLVM or hosted C/libc programs.

To build an iCEBreaker bitstream:

```sh
mov-build build/sum.mov --build-dir build/llvm-sum
```

Building does **not** flash a board. Programming requires a connected iCEBreaker and an explicit `--program`; physical-board execution has not been verified. See the [build and programming guide](docs/usage.md#reference-fpga-icebreaker).

## Documentation

| Guide | Contents |
|---|---|
| [Overview and diagrams](docs/README.md) | Project map, execution paths and source layout |
| [Usage](docs/usage.md) | LLVM/C compilation, emulator, tracing and FPGA commands |
| [Compiler](docs/compiler.md) | Supported IR, paper primitives, memory mapping and return ABI |
| [Architecture](docs/architecture.md) | MOV encoding, registers, buses, MMIO, reset and faults |
| [Formal verification](docs/formal.md) | RTL proofs, abstract models and their scope |
| [Development environment](docs/environment.md) | Combined Rust, Amaranth, electronics and formal toolchains |
| [Verification evidence](docs/verification.md) | Observed runtime, proof, diagram and bitstream checks |

The diagrams are editable Mermaid blocks and render directly on GitHub. Verification results distinguish tested executions and machine proofs from whole-compiler correctness and physical hardware operation.
