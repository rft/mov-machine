# Movsucator compatibility

Inspected upstream: [Hussdev0/movsucator](https://github.com/Hussdev0/movsucator), revision [`5cdfb470102047e603e5f5c0d96650eec42d21cc`](https://github.com/Hussdev0/movsucator/tree/5cdfb470102047e603e5f5c0d96650eec42d21cc).

## What is implemented

The assembly adapter implements the four concrete instructions in upstream [MovsucatorInstrInfo.td](https://github.com/Hussdev0/movsucator/blob/5cdfb470102047e603e5f5c0d96650eec42d21cc/lib/Target/Movsucator/MovsucatorInstrInfo.td): `MOV_rr`, `MOV_ri`, `MOV_rm`, `MOV_mr`. Those definitions print destination-first `mov dst, src`, with brackets on memory operands. The four named suffixes are also accepted for explicit forms.

Registers follow upstream [MovsucatorRegisterInfo.td](https://github.com/Hussdev0/movsucator/blob/5cdfb470102047e603e5f5c0d96650eec42d21cc/lib/Target/Movsucator/MovsucatorRegisterInfo.td): `r0`–`r3`, 32 bits. The status document mentions A/B/C/S, but those are not the concrete register names defined there; this assembler does not invent aliases.

`examples/copy.s` is a **handwritten compatibility fixture**, not claimed compiler-generated output. It exercises all four forms, labels, RAM, PC and halt. `mov-machine assemble` converts compatible textual instructions into the [local encoding](architecture.md). No upstream executable was built or run during verification.

## Why arbitrary compiler output cannot run yet

At the pinned revision:

* [MovsucatorMCCodeEmitter.cpp](https://github.com/Hussdev0/movsucator/blob/5cdfb470102047e603e5f5c0d96650eec42d21cc/lib/Target/Movsucator/MCTargetDesc/MovsucatorMCCodeEmitter.cpp) and [MovsucatorInstPrinter.cpp](https://github.com/Hussdev0/movsucator/blob/5cdfb470102047e603e5f5c0d96650eec42d21cc/lib/Target/Movsucator/MCTargetDesc/MovsucatorInstPrinter.cpp) are empty.
* [MovsucatorInstrFormats.td](https://github.com/Hussdev0/movsucator/blob/5cdfb470102047e603e5f5c0d96650eec42d21cc/lib/Target/Movsucator/MovsucatorInstrFormats.td) assigns only the opcode bits of a nominal four-byte instruction. It does not encode operand registers, addresses or a full 32-bit immediate.
* Every concrete register currently has hardware encoding zero.
* [MovsucatorExpandPseudo.cpp](https://github.com/Hussdev0/movsucator/blob/5cdfb470102047e603e5f5c0d96650eec42d21cc/lib/Target/Movsucator/MovsucatorExpandPseudo.cpp) still emits `MOV_ADD`, `MOV_NEG`, logical operations and `MOV_BR_NZ`, with comments delegating arithmetic to hardware. These are not pure data-copy MOV operations; several referenced definitions/register names are inconsistent with the concrete definitions.
* Upstream [basic-mov.ll](https://github.com/Hussdev0/movsucator/blob/5cdfb470102047e603e5f5c0d96650eec42d21cc/test/CodeGen/Movsucator/basic-mov.ll) also checks pseudo-ops and memory suffix examples inconsistent with the instruction definitions.

The FPGA therefore cannot honestly be advertised as accepting all output from this compiler. It accepts the defined pure-MOV assembly subset, not upstream objects, LLVM IR, ELF, x86 MOVfuscator output, or unresolved arithmetic/call/return/frame pseudo-ops. Unsupported instructions fail with a line-numbered error instead of being silently dropped or given invented meanings.

Complete compatibility needs an upstream backend that emits fully lowered pure-MOV sequences, a specified ABI/startup/data layout, and either compatible textual assembly or a documented binary encoding. Changing that compiler is outside this repository; the missing contract cannot be supplied by a hardware proof.

## Adapter grammar

* Registers: lowercase `r0`–`r3`; all four are writable.
* Constants: decimal or `0x` hex, optional `#` prefix, range `-2147483648` through `4294967295`.
* Labels: case-sensitive identifiers, resolve to instruction indices in immediate operands. Multiple labels can name the same instruction.
* Memory: `[rN]`, `[rN + number]`, `[rN - number]`; offsets encoded modulo 2³². No symbols/data relocations or scaled index addressing.
* Comments: `;` and `//`. `#` is an immediate prefix, not a comment.
* Accepted metadata: `.text`, `.globl`, `.global`, `.type`, `.size`, `.p2align`. Code alignment metadata does not insert padding in the local index-based format. Meaningful data/layout directives, unknown sections and unknown directives are rejected.

A compatible complete program must explicitly halt using the HALT MMIO store; MOV return/frame pseudo-ops are not a runtime. See [usage](usage.md) for assembly, execution and trace commands.
