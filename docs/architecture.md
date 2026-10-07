# Machine architecture

## ISA

Four writable 32-bit registers, `r0`–`r3`. All values and effective-address additions wrap modulo 2³². There are no arithmetic, compare, conditional-execution, call, return, or branch opcodes. Hardware address addition and sequential-PC increment are not guest arithmetic instructions.

| Opcode | Assembly | Effect |
|---|---|---|
| 0 | `mov rD, rS` | Copy register |
| 1 | `mov rD, immediate` | Copy a full 32-bit constant |
| 2 | `mov rD, [rS + offset]` | Load a 32-bit word |
| 3 | `mov [rD + offset], rS` | Store a 32-bit word |

`mov.rr`, `.ri`, `.rm`, `.mr` are accepted with the same **destination-first** operands, and their suffix must match the operand form. Negative offsets work: `mov r0, [r1 - 4]`. Memory-to-memory and immediate-to-memory moves are not instructions.

## Local executable format

A `.mov` image is a sequence of eight-byte instructions, with no header:

| Byte offset | Encoding |
|---|---|
| 0–3 | Little-endian instruction header |
| 4–7 | Little-endian extension (immediate or address offset) |

Header: bits 31–26 opcode, bits 25–24 destination register, bits 23–22 source register, bits 21–0 zero. For register copies, the extension must be zero. For immediate copies, the source-register field must be zero. Opcodes above 3 and noncanonical encodings fault.

This is this project's encoding, **not an upstream Movsucator object format**. Keeping the upstream opcode numbers does not establish binary compatibility. See [compiler compatibility](compiler.md).

The PC is a 32-bit **instruction index**, not a byte address. Labels in assembly resolve to indices. Code is immutable, separate from data RAM. There is no implicit halt at the end of an image; fetching outside it faults.

## Data address space

Default RAM: 256 zero-initialized 32-bit words, byte addresses `0x00000000`–`0x000003ff`. All data accesses must be four-byte aligned. Each access uses the pre-instruction register values, including when the load destination aliases the address register.

| Address | Read | Write |
|---|---|---|
| RAM | Load word | Store word |
| `0xfffffff8` | Fault | Halt and latch store value as `halt_code` |
| `0xfffffffc` | Current instruction index | Set PC to the store value (instruction index) |
| Other address | Fault | Fault |

MOV-to-PC supplies control flow without a branch opcode. A table lookup can select the next PC: [table-branch.s](../examples/table-branch.s). This alone is **not a proof of compiler arithmetic lowering or Turing completeness**.

## Architectural transitions

* Reset clears registers, PC, captured instruction, halted/fault status, and halt code. Core state becomes FETCH. CPU reset preserves RAM; initial FPGA configuration initializes it to zero.
* Successful instructions increment PC by one, except PC stores, which use the stored value. Successful HALT stores increment PC before stopping.
* A fault sets `halted` and `fault`, preserving PC, registers, RAM and halt code for that instruction. Faulting instructions do not retire.
* Halt/fault are sticky until reset. Stopped cores issue no requests and do not retire instructions.
* The Rust `steps` counter counts attempted instructions, including a faulting fetch/decode; it is diagnostic, not an RTL state field. Calls to `step()` after stopping do not count.

## RTL interfaces

`hw/mov_machine/core.py:MovCore` implements FETCH and EXEC phases. Its instruction bus transfers a 64-bit encoded instruction at `imem_addr` (an index). The data bus transfers a 32-bit word at `dmem_addr` (a byte address). Both complete on `valid && ready`. Read data/error are sampled at that edge. Error signals without a completed request have no effect.

Requests and architectural state remain stable while stalled. Explicit synchronous `reset` has priority and suppresses requests and retirement. The normal Amaranth `sync` domain reset also resets the core. Data-bus targets must not commit faulting stores, and must commit valid stores only on a successful completion edge. MMIO and alignment checks happen inside the core; MMIO never reaches the external data bus.

`hw/mov_machine/soc.py:MovSoC` adds synchronous block-memory-friendly ROM/RAM and a two-cycle request/completion handshake. Stores commit only at completion, so reset can cancel outstanding work. Default reference-board ROM capacity is at most 512 instructions; configurable RAM is 1–1024 words. The emulator can use larger images and RAM within the 32-bit non-MMIO address space; FPGA capacity and place/route are the deployment limit.

Architectural debug signals: `pc`, `regs`, `halted`, `fault`, `halt_code`; execution debug: `state`, `instruction`, `retire`. The core is board-neutral; the reference board wrapper synchronizes its reset button and drives status LEDs.
