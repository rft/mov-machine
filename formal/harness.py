"""Full 32/64-bit RTL properties; no restrictions on bus inputs or reset.

The reference never reads a DUT decode or control signal. Its complete state is
kept separate so induction proves the generated core refines these transitions.
Bus data need not remain stable before ready: it is sampled only at completion.
The external memory is responsible for address mapping, reported via error.
"""
from amaranth import Array, Assert, Cover, Elaboratable, Module, Mux, Signal
from amaranth.asserts import AnySeq

from mov_machine.core import MovCore
from mov_machine.soc import MovSoC


class ArchitecturalSpec(Elaboratable):
    def __init__(self, bus):
        self.bus = bus
        self.pc = Signal(32)
        self.regs = [Signal(32, name=f"spec_r{i}") for i in range(4)]
        self.halted = Signal()
        self.fault = Signal()
        self.halt_code = Signal(32)
        self.phase = Signal()
        self.word = Signal(64)
        self.fetch = Signal()
        self.memory = Signal()
        self.write = Signal()
        self.address = Signal(32)
        self.value = Signal(32)
        self.retire = Signal()

    def elaborate(self, platform):
        m = Module()
        b = self.bus
        r = Array(self.regs)
        # This is the ISA specification, not an import of implementation decode.
        opcode = self.word[26:32]
        destination = self.word[24:26]
        source = self.word[22:24]
        immediate = self.word[32:64]
        canonical = ((self.word[:22] == 0) & (opcode < 4)
                     & ((opcode != 0) | (immediate == 0))
                     & ((opcode != 1) | (source == 0)))
        pointer = Mux(opcode == 2, r[source], r[destination])
        effective = Signal(32)
        m.d.comb += effective.eq(pointer + immediate)
        aligned = effective[:2] == 0
        pc_port = effective == 0xFFFFFFFC
        halt_port = effective == 0xFFFFFFF8
        running = ~b.reset & ~self.halted
        m.d.comb += [
            self.fetch.eq(running & ~self.phase),
            self.memory.eq(running & self.phase & canonical & (opcode >= 2)
                           & aligned & ~pc_port & ~halt_port),
            self.write.eq(opcode == 3), self.address.eq(effective),
            self.value.eq(r[source]), self.retire.eq(0),
        ]
        # Independent next-state equations: defaults preserve EVERY state item.
        next_pc = Signal(32)
        next_regs = [Signal(32, name=f"next_r{i}") for i in range(4)]
        next_halted, next_fault = Signal(), Signal()
        next_code, next_phase, next_word = Signal(32), Signal(), Signal(64)
        m.d.comb += [next_pc.eq(self.pc), next_halted.eq(self.halted),
                     next_fault.eq(self.fault), next_code.eq(self.halt_code),
                     next_phase.eq(self.phase), next_word.eq(self.word)]
        m.d.comb += [n.eq(old) for n, old in zip(next_regs, self.regs)]
        fail = Signal()
        commit = Signal()
        register_write = Signal()
        register_value = Signal(32)
        m.d.comb += [fail.eq(0), commit.eq(0), register_write.eq(0),
                     register_value.eq(0)]
        with m.If(running):
            with m.If(~self.phase):
                with m.If(b.imem_ready):
                    with m.If(b.imem_error):
                        m.d.comb += fail.eq(1)
                    with m.Else():
                        m.d.comb += [next_word.eq(b.imem_data), next_phase.eq(1)]
            with m.Else():
                with m.If(~canonical):
                    m.d.comb += fail.eq(1)
                with m.Elif(opcode < 2):
                    m.d.comb += [commit.eq(1), register_write.eq(1),
                                 register_value.eq(Mux(opcode == 0, r[source], immediate))]
                with m.Elif(~aligned):
                    m.d.comb += fail.eq(1)
                with m.Elif(pc_port):
                    m.d.comb += commit.eq(1)
                    with m.If(opcode == 2):
                        m.d.comb += [register_write.eq(1), register_value.eq(self.pc)]
                with m.Elif(halt_port):
                    with m.If(opcode == 2):
                        m.d.comb += fail.eq(1)
                    with m.Else():
                        m.d.comb += [commit.eq(1), next_halted.eq(1), next_code.eq(r[source])]
                with m.Elif(b.dmem_ready):
                    with m.If(b.dmem_error):
                        m.d.comb += fail.eq(1)
                    with m.Else():
                        m.d.comb += commit.eq(1)
                        with m.If(opcode == 2):
                            m.d.comb += [register_write.eq(1), register_value.eq(b.dmem_rdata)]
        with m.If(commit):
            m.d.comb += [next_phase.eq(0), self.retire.eq(1),
                         next_pc.eq(Mux((opcode == 3) & pc_port, r[source], self.pc + 1))]
        with m.If(register_write):
            for i in range(4):
                with m.If(destination == i):
                    m.d.comb += next_regs[i].eq(register_value)
        with m.If(fail):
            m.d.comb += [next_halted.eq(1), next_fault.eq(1)]
        with m.If(b.reset):
            m.d.comb += [next_pc.eq(0), next_halted.eq(0), next_fault.eq(0),
                         next_code.eq(0), next_phase.eq(0), next_word.eq(0)]
            m.d.comb += [n.eq(0) for n in next_regs]
        m.d.sync += [self.pc.eq(next_pc), self.halted.eq(next_halted),
                     self.fault.eq(next_fault), self.halt_code.eq(next_code),
                     self.phase.eq(next_phase), self.word.eq(next_word)]
        m.d.sync += [old.eq(n) for old, n in zip(self.regs, next_regs)]
        return m


def check_core(m, core, name):
    spec = ArchitecturalSpec(core)
    m.submodules[name] = spec
    m.d.comb += [Assert(core.pc == spec.pc), Assert(core.halted == spec.halted),
                 Assert(core.fault == spec.fault), Assert(core.halt_code == spec.halt_code),
                 Assert(core.state == spec.phase), Assert(core.instruction == spec.word),
                 Assert(core.imem_addr == spec.pc), Assert(core.imem_valid == spec.fetch),
                 Assert(core.dmem_valid == spec.memory), Assert(core.retire == spec.retire)]
    m.d.comb += [Assert(actual == expected) for actual, expected in zip(core.regs, spec.regs)]
    with m.If(spec.memory):
        m.d.comb += [Assert(core.dmem_addr == spec.address),
                     Assert(core.dmem_write == spec.write), Assert(core.dmem_wdata == spec.value)]
    return spec


class CoreFormal(Elaboratable):
    def elaborate(self, platform):
        m = Module()
        m.submodules.dut = c = MovCore()
        for signal in (c.reset, c.imem_ready, c.imem_data, c.imem_error,
                       c.dmem_ready, c.dmem_rdata, c.dmem_error):
            m.d.comb += signal.eq(AnySeq(len(signal)))
        check_core(m, c, "reference")
        op = c.instruction[26:32]
        ext = c.instruction[32:64]
        # Every Cover is solved separately by SBY, so unreachable behavior fails.
        for opcode in range(4):
            m.d.comb += Cover(c.retire & (op == opcode))
        m.d.comb += [
            Cover(c.imem_valid & ~c.imem_ready & c.imem_error),
            Cover(c.dmem_valid & ~c.dmem_ready & c.dmem_error),
            Cover(c.imem_valid & c.imem_ready & c.imem_error),
            Cover(c.dmem_valid & c.dmem_ready & c.dmem_error),
            Cover(c.retire & (op == 2) & (c.dmem_addr == 0xFFFFFFFC)),
            Cover(c.retire & (op == 3) & (c.dmem_addr == 0xFFFFFFFC)),
            Cover(c.retire & (c.pc == 0xFFFFFFFF)),
            Cover(c.halted & ~c.fault & (c.halt_code != 0)),
            Cover(c.state & ~c.halted & (op >= 4)),
            Cover(c.state & ~c.halted & (c.instruction[:22] != 0)),
            Cover(c.state & ~c.halted & (op == 0) & (ext != 0)),
            Cover(c.state & ~c.halted & (op == 1) & (c.instruction[22:24] != 0)),
            Cover(c.state & ~c.halted & (op == 2) & (c.dmem_addr == 0xFFFFFFF8)),
            Cover(c.state & ~c.halted & (op >= 2) & (c.dmem_addr[:2] != 0)),
        ]
        fetch_stalled, data_stalled, stopped = Signal(), Signal(), Signal()
        m.d.sync += [fetch_stalled.eq(c.imem_valid & ~c.imem_ready),
                     data_stalled.eq(c.dmem_valid & ~c.dmem_ready), stopped.eq(c.halted)]
        m.d.comb += [Cover(fetch_stalled & c.reset), Cover(data_stalled & c.reset),
                     Cover(stopped & c.reset), Cover(stopped & c.halted & ~c.reset)]
        return m


def instruction(op, dst=0, src=0, ext=0):
    return (op << 26) | (dst << 24) | (src << 22) | ((ext & 0xFFFFFFFF) << 32)


# Concrete integration scopes: two RAM words, four distinct immutable ROMs.
# Main path: store/read word1, load PC, jump over an illegal instruction, halt.
SOC_PROGRAMS = {
    "roundtrip": [instruction(1, 0, ext=0x89ABCDEF),
                  instruction(3, 1, 0, 4), instruction(2, 2, 1, 4),
                  instruction(2, 3, 1, 0xFFFFFFFC), instruction(1, 3, ext=7),
                  instruction(3, 1, 3, 0xFFFFFFFC), instruction(63),
                  instruction(3, 1, 2, 0xFFFFFFF8)],
    "bad_store": [instruction(1, 0, ext=0x11223344), instruction(3, 1, 0, 8)],
    "bad_load": [instruction(2, 0, 1, 8)],
    "bad_fetch": [instruction(1, 0, ext=0x11223344)],
}


class SoCFormal(Elaboratable):
    def elaborate(self, platform):
        m = Module()
        reset = Signal()
        m.d.comb += reset.eq(AnySeq(1))
        for name, program in SOC_PROGRAMS.items():
            soc = MovSoC(program, ram_words=2)
            m.submodules[name] = soc
            c = soc.core
            m.d.comb += soc.reset.eq(reset)
            check_core(m, c, name + "_reference")
            fetch_pending, data_pending = Signal(), Signal()
            fetch_bad, data_bad = Signal(), Signal()
            fetch_word, load_word = Signal(64), Signal(32)
            # RAM is initialized on configuration, not erased by CPU/domain reset.
            ram = [Signal(32, reset_less=True, name=f"{name}_ram{i}") for i in range(2)]
            for i in range(2):
                observe = soc.ram.read_port(domain="comb")
                m.d.comb += [observe.addr.eq(i), Assert(observe.data == ram[i])]
            m.d.comb += [Assert(c.imem_ready == (fetch_pending & ~reset)),
                         Assert(c.dmem_ready == (data_pending & ~reset))]
            with m.If(c.imem_valid & c.imem_ready):
                m.d.comb += Assert(c.imem_error == fetch_bad)
                with m.If(~fetch_bad):
                    m.d.comb += Assert(c.imem_data == fetch_word)
            with m.If(c.dmem_valid & c.dmem_ready):
                m.d.comb += Assert(c.dmem_error == data_bad)
                with m.If(~data_bad & ~c.dmem_write):
                    m.d.comb += Assert(c.dmem_rdata == load_word)
            with m.If(reset):
                m.d.sync += [fetch_pending.eq(0), data_pending.eq(0),
                             fetch_bad.eq(0), data_bad.eq(0)]
            with m.Else():
                m.d.sync += [fetch_pending.eq(c.imem_valid & ~fetch_pending),
                             data_pending.eq(c.dmem_valid & ~data_pending)]
                with m.If(c.imem_valid & ~fetch_pending):
                    m.d.sync += fetch_bad.eq(c.imem_addr >= len(program))
                    for i, word in enumerate(program):
                        with m.If(c.imem_addr == i):
                            m.d.sync += fetch_word.eq(word)
                with m.If(c.dmem_valid & ~data_pending):
                    m.d.sync += data_bad.eq(c.dmem_addr >= 8)
                    for i in range(2):
                        with m.If(c.dmem_addr == 4 * i):
                            m.d.sync += load_word.eq(ram[i])
            for i in range(2):
                with m.If(c.dmem_valid & c.dmem_ready & c.dmem_write
                          & (c.dmem_addr == 4 * i)):
                    m.d.sync += ram[i].eq(c.dmem_wdata)
            if name == "roundtrip":
                m.d.comb += [Cover(c.halted & ~c.fault & (c.halt_code == 0x89ABCDEF)),
                             Cover(reset & (ram[1] == 0x89ABCDEF))]
            else:
                m.d.comb += Cover(c.halted & c.fault)
        return m
