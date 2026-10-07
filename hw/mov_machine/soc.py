from amaranth import Elaboratable, Module, Signal, unsigned
from amaranth.lib.memory import Memory

from .core import MovCore




class MovSoC(Elaboratable):
    """Core with synchronous ROM and RAM, suitable for FPGA block memories."""

    def __init__(self, program, ram_words=256):
        if not program or len(program) > 16384:
            raise ValueError("reference FPGA ROM supports 1..16384 instructions; physical fit is checked by place/route")
        if not 1 <= ram_words <= 1024:
            raise ValueError("reference FPGA RAM supports 1..1024 words")
        if any(not 0 <= word < (1 << 64) for word in program):
            raise ValueError("instructions must fit in 64 bits")
        self.core = MovCore()
        self.reset = Signal()
        self.rom = Memory(shape=unsigned(64), depth=len(program), init=program)
        self.ram = Memory(shape=unsigned(32), depth=ram_words, init=[])
        self.program_size = len(program)
        self.ram_words = ram_words

    def elaborate(self, platform):
        m = Module()
        m.submodules.core = core = self.core
        m.submodules.rom = self.rom
        m.submodules.ram = self.ram
        rom_read = self.rom.read_port()
        ram_read = self.ram.read_port()
        ram_write = self.ram.write_port()
        fetch_pending = Signal()
        fetch_error = Signal()
        data_pending = Signal()
        data_error = Signal()
        fetch_issue = Signal()
        data_issue = Signal()
        m.d.comb += [
            core.reset.eq(self.reset),
            fetch_issue.eq(core.imem_valid & ~fetch_pending),
            rom_read.addr.eq(core.imem_addr),
            rom_read.en.eq(fetch_issue & (core.imem_addr < self.program_size)),
            core.imem_ready.eq(fetch_pending & ~self.reset),
            core.imem_data.eq(rom_read.data),
            core.imem_error.eq(fetch_error),
            data_issue.eq(core.dmem_valid & ~data_pending),
            ram_read.addr.eq(core.dmem_addr[2:]),
            ram_read.en.eq(data_issue & ~core.dmem_write
                           & (core.dmem_addr < self.ram_words * 4)),
            core.dmem_ready.eq(data_pending & ~self.reset),
            core.dmem_rdata.eq(ram_read.data),
            core.dmem_error.eq(data_error),
            ram_write.addr.eq(core.dmem_addr[2:]),
            ram_write.data.eq(core.dmem_wdata),
            # Stores take effect only on completion, never on request issue.
            ram_write.en.eq(core.dmem_valid & core.dmem_ready
                            & core.dmem_write & ~data_error),
        ]
        with m.If(self.reset):
            m.d.sync += [fetch_pending.eq(0), data_pending.eq(0),
                         fetch_error.eq(0), data_error.eq(0)]
        with m.Else():
            with m.If(fetch_pending):
                m.d.sync += fetch_pending.eq(0)
            with m.Elif(fetch_issue):
                m.d.sync += [fetch_pending.eq(1),
                             fetch_error.eq(core.imem_addr >= self.program_size)]
            with m.If(data_pending):
                m.d.sync += data_pending.eq(0)
            with m.Elif(data_issue):
                m.d.sync += [data_pending.eq(1),
                             data_error.eq(core.dmem_addr >= self.ram_words * 4)]
        return m
