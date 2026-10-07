from amaranth import Array, Elaboratable, Module, Mux, Signal

PC_ADDRESS = 0xFFFFFFFC
HALT_ADDRESS = 0xFFFFFFF8
FETCH = 0
EXEC = 1


class MovCore(Elaboratable):
    """Two-phase 32-bit MOV core; buses complete on valid && ready.

    Instruction addresses are indices, data addresses are bytes. Read data and
    error are sampled on the completion edge. Reset cancels outstanding work.
    """

    def __init__(self):
        self.reset = Signal()
        self.imem_valid = Signal()
        self.imem_addr = Signal(32)
        self.imem_ready = Signal()
        self.imem_data = Signal(64)
        self.imem_error = Signal()
        self.dmem_valid = Signal()
        self.dmem_write = Signal()
        self.dmem_addr = Signal(32)
        self.dmem_wdata = Signal(32)
        self.dmem_ready = Signal()
        self.dmem_rdata = Signal(32)
        self.dmem_error = Signal()
        self.pc = Signal(32)
        self.regs = [Signal(32, name=f"r{i}") for i in range(4)]
        self.halted = Signal()
        self.fault = Signal()
        self.halt_code = Signal(32)
        self.state = Signal(init=FETCH)
        self.instruction = Signal(64)
        self.retire = Signal()

    def ports(self):
        return [self.reset, self.imem_valid, self.imem_addr, self.imem_ready,
                self.imem_data, self.imem_error, self.dmem_valid,
                self.dmem_write, self.dmem_addr, self.dmem_wdata,
                self.dmem_ready, self.dmem_rdata, self.dmem_error, self.pc,
                *self.regs, self.halted, self.fault, self.halt_code,
                self.state, self.instruction, self.retire]

    def elaborate(self, platform):
        m = Module()
        regs = Array(self.regs)
        op = self.instruction[26:32]
        dst = self.instruction[24:26]
        src = self.instruction[22:24]
        ext = self.instruction[32:64]
        legal = Signal()
        address = Signal(32)
        active = Signal()
        external = Signal()
        m.d.comb += [
            legal.eq((op <= 3) & (self.instruction[:22] == 0)
                     & ((op != 0) | (ext == 0))
                     & ((op != 1) | (src == 0))),
            address.eq(Mux(op == 3, regs[dst], regs[src]) + ext),
            active.eq(~self.reset & ~self.halted),
            external.eq((address != PC_ADDRESS) & (address != HALT_ADDRESS)),
            self.imem_valid.eq(active & (self.state == FETCH)),
            self.imem_addr.eq(self.pc),
            self.dmem_valid.eq(active & (self.state == EXEC) & legal
                               & (op >= 2) & (address[:2] == 0) & external),
            self.dmem_write.eq(op == 3),
            self.dmem_addr.eq(address),
            self.dmem_wdata.eq(regs[src]),
            self.retire.eq(0),
        ]

        with m.If(self.reset):
            m.d.sync += [self.pc.eq(0), self.state.eq(FETCH),
                         self.instruction.eq(0), self.halted.eq(0),
                         self.fault.eq(0), self.halt_code.eq(0)]
            m.d.sync += [reg.eq(0) for reg in self.regs]
        with m.Elif(~self.halted):
            with m.If(self.state == FETCH):
                with m.If(self.imem_ready):
                    with m.If(self.imem_error):
                        m.d.sync += [self.halted.eq(1), self.fault.eq(1)]
                    with m.Else():
                        m.d.sync += [self.instruction.eq(self.imem_data),
                                     self.state.eq(EXEC)]
            with m.Else():
                with m.If(~legal):
                    m.d.sync += [self.halted.eq(1), self.fault.eq(1)]
                with m.Elif(op <= 1):
                    m.d.comb += self.retire.eq(1)
                    m.d.sync += [regs[dst].eq(Mux(op == 0, regs[src], ext)),
                                 self.pc.eq(self.pc + 1), self.state.eq(FETCH)]
                with m.Elif(address[:2] != 0):
                    m.d.sync += [self.halted.eq(1), self.fault.eq(1)]
                with m.Elif(address == PC_ADDRESS):
                    m.d.comb += self.retire.eq(1)
                    m.d.sync += self.state.eq(FETCH)
                    with m.If(op == 3):
                        m.d.sync += self.pc.eq(regs[src])
                    with m.Else():
                        m.d.sync += [regs[dst].eq(self.pc), self.pc.eq(self.pc + 1)]
                with m.Elif(address == HALT_ADDRESS):
                    with m.If(op == 3):
                        m.d.comb += self.retire.eq(1)
                        m.d.sync += [self.halted.eq(1), self.halt_code.eq(regs[src]),
                                     self.pc.eq(self.pc + 1), self.state.eq(FETCH)]
                    with m.Else():
                        m.d.sync += [self.halted.eq(1), self.fault.eq(1)]
                with m.Elif(self.dmem_ready):
                    with m.If(self.dmem_error):
                        m.d.sync += [self.halted.eq(1), self.fault.eq(1)]
                    with m.Else():
                        m.d.comb += self.retire.eq(1)
                        m.d.sync += [self.pc.eq(self.pc + 1), self.state.eq(FETCH)]
                        with m.If(op == 2):
                            m.d.sync += regs[dst].eq(self.dmem_rdata)
        return m
