module mov_machine

// Exact finite, ONE-STEP relational abstraction, NOT the full-width RTL proof.
// Two registers, two RAM words, four values/PCs, five abstract addresses.
// Address selection abstracts effective-address calculation and alignment.
// Before can be any typed state satisfying fault=>halted; no reachable-state
// restriction. Commands check preservation and move semantics of this relation.
abstract sig Bool {}
one sig Yes, No extends Bool {}
abstract sig Reg {}
one sig R0, R1 extends Reg {}
abstract sig Word {}
one sig W0, W1, W2, W3 extends Word {}
abstract sig Addr {}
one sig RAM0, RAM1, PC, HALT, Invalid extends Addr {}
abstract sig Action {}
one sig Reset, Stopped, Stall, Fetch, FetchError, RR, RI, Load, Store,
        BadInstruction, MemoryError extends Action {}
sig State {
  pc: one Word,
  regs: Reg -> one Word,
  ram: (RAM0 + RAM1) -> one Word,
  halted, fault, executing: one Bool,
  code: one Word
}
one sig Step {
  pre, post: one State,
  action: one Action,
  dst, src: one Reg,
  imm: one Word,
  address: one Addr
}
fun successor[w: Word]: one Word {
  w.(W0->W1 + W1->W2 + W2->W3 + W3->W0)
}
pred sameArchitecture[b, a: State] {
  a.pc=b.pc and a.regs=b.regs and a.ram=b.ram and
  a.halted=b.halted and a.fault=b.fault and a.code=b.code
}
pred fail[b, a: State] {
  a.pc=b.pc and a.regs=b.regs and a.ram=b.ram and a.code=b.code and
  a.executing=b.executing and a.halted=Yes and a.fault=Yes
}
pred complete[b, a: State] {
  a.executing=No and a.fault=b.fault
}
pred transition {
  let b=Step.pre, a=Step.post, op=Step.action |
    b != a and (b.fault=Yes implies b.halted=Yes) and {
      op=Reset implies {
        a.pc=W0 and a.regs=Reg->W0 and a.ram=b.ram and
        a.halted=No and a.fault=No and a.code=W0 and a.executing=No
      }
      op=Stopped implies { b.halted=Yes and sameArchitecture[b,a] and a.executing=b.executing }
      op=Stall implies { b.halted=No and sameArchitecture[b,a] and a.executing=b.executing }
      op=Fetch implies {
        b.halted=No and b.executing=No and sameArchitecture[b,a] and a.executing=Yes
      }
      op=FetchError implies { b.halted=No and b.executing=No and fail[b,a] }
      op in RR+RI+Load+Store+BadInstruction+MemoryError implies {
        b.halted=No and b.executing=Yes
      }
      op=BadInstruction implies fail[b,a]
      op=MemoryError implies { Step.address in RAM0+RAM1 and fail[b,a] }
      op=RR implies {
        complete[b,a] and a.pc=successor[b.pc] and
        a.regs=b.regs ++ Step.dst->b.regs[Step.src] and
        a.ram=b.ram and a.halted=b.halted and a.code=b.code
      }
      op=RI implies {
        complete[b,a] and a.pc=successor[b.pc] and
        a.regs=b.regs ++ Step.dst->Step.imm and
        a.ram=b.ram and a.halted=b.halted and a.code=b.code
      }
      op=Load implies {
        Step.address in HALT+Invalid implies fail[b,a]
        Step.address not in HALT+Invalid implies {
          complete[b,a] and a.pc=successor[b.pc] and
          a.regs=b.regs ++ Step.dst->(Step.address=PC => b.pc else b.ram[Step.address]) and
          a.ram=b.ram and a.halted=b.halted and a.code=b.code
        }
      }
      op=Store implies {
        Step.address=Invalid implies fail[b,a]
        Step.address!=Invalid implies {
          complete[b,a] and a.regs=b.regs and
          a.pc=(Step.address=PC => b.regs[Step.src] else successor[b.pc]) and
          a.ram=(Step.address in RAM0+RAM1 => b.ram ++ Step.address->b.regs[Step.src] else b.ram) and
          a.halted=(Step.address=HALT => Yes else b.halted) and
          a.code=(Step.address=HALT => b.regs[Step.src] else b.code)
        }
      }
    }
}
fact Machine { transition }
assert FaultImpliesStop { Step.post.fault=Yes implies Step.post.halted=Yes }
assert PreciseFault {
  let b=Step.pre, a=Step.post |
    (a.fault=Yes and b.fault=No) implies
      a.pc=b.pc and a.regs=b.regs and a.ram=b.ram and a.code=b.code
}
assert StopSticky {
  Step.pre.halted=Yes and Step.action!=Reset implies
    sameArchitecture[Step.pre,Step.post]
}
assert RegisterFrame {
  let b=Step.pre, a=Step.post |
    Step.action in RR+RI+Load implies
      all r: Reg-Step.dst | a.regs[r]=b.regs[r]
}
assert MemoryFrame {
  Step.action!=Store implies Step.post.ram=Step.pre.ram
}
assert PCTransition {
  let b=Step.pre, a=Step.post |
    Step.action in RR+RI or
    (Step.action=Load and Step.address not in HALT+Invalid) or
    (Step.action=Store and Step.address not in PC+Invalid) implies a.pc=successor[b.pc]
}
assert ResetPreservesRAM {
  Step.action=Reset implies Step.post.ram=Step.pre.ram and
    Step.post.pc=W0 and Step.post.regs=Reg->W0 and Step.post.halted=No and Step.post.fault=No
}
assert MoveSemantics {
  let b=Step.pre, a=Step.post |
    (Step.action=RR implies a.regs[Step.dst]=b.regs[Step.src]) and
    (Step.action=RI implies a.regs[Step.dst]=Step.imm) and
    (Step.action=Load and Step.address in RAM0+RAM1 implies a.regs[Step.dst]=b.ram[Step.address]) and
    (Step.action=Load and Step.address=PC implies a.regs[Step.dst]=b.pc) and
    (Step.action=Store and Step.address in RAM0+RAM1 implies a.ram[Step.address]=b.regs[Step.src]) and
    (Step.action=Store and Step.address=PC implies a.pc=b.regs[Step.src]) and
    (Step.action=Store and Step.address=HALT implies a.halted=Yes and a.code=b.regs[Step.src])
}
// Exactly two distinct states; enum signatures above fix every other atom count.
check FaultImpliesStop for 12 but exactly 2 State expect 0
check PreciseFault for 12 but exactly 2 State expect 0
check StopSticky for 12 but exactly 2 State expect 0
check RegisterFrame for 12 but exactly 2 State expect 0
check MemoryFrame for 12 but exactly 2 State expect 0
check PCTransition for 12 but exactly 2 State expect 0
check ResetPreservesRAM for 12 but exactly 2 State expect 0
check MoveSemantics for 12 but exactly 2 State expect 0
// Non-vacuity: each action must have at least one allowed finite transition.
run CanReset { Step.action=Reset } for 12 but exactly 2 State expect 1
run CanStop { Step.action=Stopped } for 12 but exactly 2 State expect 1
run CanStall { Step.action=Stall } for 12 but exactly 2 State expect 1
run CanFetch { Step.action=Fetch } for 12 but exactly 2 State expect 1
run CanFetchFault { Step.action=FetchError } for 12 but exactly 2 State expect 1
run CanRR { Step.action=RR } for 12 but exactly 2 State expect 1
run CanRI { Step.action=RI } for 12 but exactly 2 State expect 1
run CanLoad { Step.action=Load and Step.address=RAM1 } for 12 but exactly 2 State expect 1
run CanStore { Step.action=Store and Step.address=RAM1 } for 12 but exactly 2 State expect 1
run CanJump { Step.action=Store and Step.address=PC } for 12 but exactly 2 State expect 1
run CanHalt { Step.action=Store and Step.address=HALT } for 12 but exactly 2 State expect 1
run CanBadInstruction { Step.action=BadInstruction } for 12 but exactly 2 State expect 1
run CanMemoryFault { Step.action=MemoryError } for 12 but exactly 2 State expect 1
