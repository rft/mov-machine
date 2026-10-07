# Verification evidence

Observed with the locked development shell and source in this repository. These results are specific to the indicated design/program/configuration; rerun checks after changes.

## Runtime and integration

* Combined shell: Python imports of Amaranth, amaranth-boards, SKiDL, kinparse and matplotlib succeeded. Rust, Yosys, sby, Z3, TLC, Alloy 6, nextpnr-ice40, icepack and project wrappers were available.
* Rust suite: 10 behavioral tests passed (8 machine/assembler, 2 actual CLI). Boundaries include malformed images/instructions, alias loads, wrapped addresses, RAM limits, MMIO, explicit halt, reset, step budget and rejected pseudo-ops.
* Rust formatting and `cargo clippy --all-targets --locked -- -D warnings` completed successfully in the locked shell.
* Differential smoke: 46 programs compared instruction by instruction between the Rust executable and both the synchronous-memory Amaranth SoC and a core with randomized bus stalls. This includes 32 fixed-seed generated programs, two executable examples, memory/MMIO/fault boundaries and four malformed encodings.
* Reset smoke: a pending store was cancelled; previous RAM contents survived; the processor restarted and completed successfully.
* Actual CLI execution of `examples/copy.s`: PC 11, registers `[42,9,42,4294967288]`, successful halt code 42 after 10 attempted instructions. The trace included RAM store, load, register copy, PC jump and HALT store.
* Board-neutral RTLIL and Verilog exports succeeded.

The checked examples are handwritten, not captured upstream compiler output. No claim is made that an upstream LLVM backend was successfully compiled or executed.

## FPGA build

`mov-build build/copy.mov`, default 256-word RAM, completed Yosys synthesis, nextpnr-ice40 place/route and icepack bitstream creation:

* Output: `build/icebreaker/mov_machine.bin`.
* Device: iCE40UP5K-SG48.
* Logic cells: 810 / 5280 (15%).
* Block RAMs: 2 / 30 (6%).
* nextpnr reported clock maximum 20.59 MHz, **PASS at the configured 12.00 MHz**.
* Supporting reports: `build/icebreaker/mov_machine.rpt` and `mov_machine.tim`.

This is a tool-estimated timing/resource result for this image and capacity, not a measured FPGA result. No physical board was connected or programmed. Other images/capacities need their own build/timing check.

## Formal observations

* Full-width core: SymbiYosys/Z3 base cases and temporal induction passed; core cover task passed.
* Concrete SoC integration: SymbiYosys/Z3 base cases and temporal induction passed for all four two-word-RAM configurations; all integration covers passed, including successful roundtrip/jump/HALT and reset-persistent RAM. The successful halt witness reached step 23.
* TLA+/TLC: no invariant error; 4,323,397 generated states, 411,233 distinct states, complete finite search depth 19, queue empty.
* Alloy 6: eight assertions UNSAT (no counterexamples within scope), all thirteen non-vacuity runs SAT; runner validated solver receipt.

Formal source/contracts and finite-model bounds: [formal.md](formal.md). RTL induction does not prove physical hardware, compiler correctness or arbitrary-program termination; TLA+/Alloy results apply only to their explicit abstractions.
