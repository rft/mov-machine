# Verification evidence

Observed with the locked development shell and source in this repository. These results are specific to the indicated design/program/configuration; rerun checks after changes.

## Runtime and integration

* Combined shell: Python imports of Amaranth, amaranth-boards, SKiDL, kinparse and matplotlib succeeded. Rust, Clang/LLVM, Yosys, sby, Z3, TLC, Alloy 6, nextpnr-ice40, icepack and project wrappers were available.
* Rust suite: 59 behavioral tests passed: 30 LLVM frontend, 16 compiler, 8 machine/assembler, 3 executable/result ABI, 2 actual CLI. Compiler cases exercise variable i1/i8/i16/i32 arithmetic/comparisons, signed division/remainder, shifts, casts, PHI swaps, calls, globals, byte aliasing, pointer roundtrips, bounds faults and reset initialization. Frontend cases include actual Clang-like syntax, multiline IR, CFG/dominance/type checks and unsupported inputs.
* Rust formatting and `cargo clippy --all-targets --locked -- -D warnings` completed successfully in the locked shell.
* Native differential smoke: 46 programs compared instruction by instruction between the Rust executable and both the synchronous-memory Amaranth SoC and a core with randomized bus stalls. This includes 32 fixed-seed generated programs, two executable examples, memory/MMIO/fault boundaries and four malformed encodings.
* Reset smoke: a pending native store was cancelled, previous RAM contents survived, and the processor restarted successfully. A separate actual-SoC compiler smoke ran the global-memory program twice across CPU reset: both returned 42, proving startup restored its mutated global.
* Actual CLI execution of `examples/copy.s`: PC 11, registers `[42,9,42,4294967288]`, successful halt code 42 after 10 attempted instructions. The trace included RAM store, load, register copy, PC jump and HALT store.
* Board-neutral RTLIL and Verilog exports succeeded.
* Documentation-only smoke: all 10 Mermaid diagrams rendered as SVG with Mermaid 11.4.1 in Chromium and were visually inspected; no syntax-error diagrams or out-of-bounds node/edge labels were observed. All 20 local Markdown links resolved, including referenced heading anchors. Rendering tools were temporary; no project runtime/toolchain dependencies changed.

### Compiler-generated execution

`mov-check` verified the LLVM fixtures with `opt`, compared their returns with `lli`, compiled actual i386 Clang output, and compared its return with native C execution. Every generated MOV transition and the decoded result RAM matched both the synchronous SoC and a randomized-stall core:

| Input | Decoded result | MOV instructions | Executed MOV steps | Required RAM words |
|---|---:|---:|---:|---:|
| `examples/llvm/sum.ll` (inline variable addition) | 42 | 721 | 721 | 112 |
| `examples/llvm/loop.ll` (loop/PHIs) | 55 | 1307 | 10821 | 115 |
| `examples/llvm/memory.ll` (global/dynamic GEP/load/store) | 42 | 10209 | 10197 | 646 |
| Clang `sum.c`, entry `sum`, arguments 17/25 | 42 | 532 | 532 | 48 |

Direct `exec-ir` CLI smoke additionally returned 55 for the loop and zero for variable i8 addition 255 + 1. Actual CLI commands rejected insufficient image RAM, missing entry arguments and unsupported i64 IR with exit status 2. The memory fixture uses 32 logical source bytes; arithmetic-only fixtures allocate no source-memory tree.


LLVM/native comparisons check source return values; trace comparisons additionally check MOV-level state and result memory. These finite executions are not an all-program LLVM equivalence theorem.

## FPGA build

### Compiler-generated LLVM program

`mov-build build/sum.mov --build-dir build/llvm-sum` completed synthesis, place/route and bitstream creation for the 721-instruction LLVM sum, using default 256-word RAM:

* Output: `build/llvm-sum/mov_machine.bin`.
* Device: iCE40UP5K-SG48.
* Logic cells: 887 / 5280.
* Block RAMs: 9 / 30.
* Final nextpnr clock maximum: 20.13 MHz, **PASS at 12.00 MHz**.
* Reports: `build/llvm-sum/mov_machine.rpt` and `mov_machine.tim`.
* Emulator result: 42; architectural PC/steps 721, halt code 264 (encoded result address), no fault.

### Earlier native assembly fixture


`mov-build build/copy.mov`, default 256-word RAM, completed Yosys synthesis, nextpnr-ice40 place/route and icepack bitstream creation:

* Output: `build/icebreaker/mov_machine.bin`.
* Device: iCE40UP5K-SG48.
* Logic cells: 810 / 5280 (15%).
* Block RAMs: 2 / 30 (6%).
* nextpnr reported clock maximum 20.59 MHz, **PASS at the configured 12.00 MHz**.
* Supporting reports: `build/icebreaker/mov_machine.rpt` and `mov_machine.tim`.

This is a tool-estimated timing/resource result for this image and capacity, not a measured FPGA result. No physical board was connected or programmed. Other images/capacities need their own build/timing check.

## Formal observations

These ISA-machine proof runs preceded the LLVM compiler addition. That addition did not change the ISA/state-transition RTL; it changed the executable loader and accepted SoC ROM-size bound. No new whole-compiler proof is claimed.


* Full-width core: SymbiYosys/Z3 base cases and temporal induction passed; core cover task passed.
* Concrete SoC integration: SymbiYosys/Z3 base cases and temporal induction passed for all four two-word-RAM configurations; all integration covers passed, including successful roundtrip/jump/HALT and reset-persistent RAM. The successful halt witness reached step 23.
* TLA+/TLC: no invariant error; 4,323,397 generated states, 411,233 distinct states, complete finite search depth 19, queue empty.
* Alloy 6: eight assertions UNSAT (no counterexamples within scope), all thirteen non-vacuity runs SAT; runner validated solver receipt.

Formal source/contracts and finite-model bounds: [formal.md](formal.md). RTL induction does not prove physical hardware, compiler correctness or arbitrary-program termination; TLA+/Alloy results apply only to their explicit abstractions.
