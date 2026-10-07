---------------------------- MODULE MovMachine ----------------------------
EXTENDS Integers, Naturals, FiniteSets, TLC

\* Finite abstraction ONLY: 2 registers, 2 RAM words, 2-bit values and PC.
\* Address selection abstracts base+offset/alignment; "bad" includes all
\* unmapped/misaligned addresses. No compiler, RTL or 32-bit proof is claimed.
Registers == 0..1
Values == 0..3
Addresses == {"ram0", "ram1", "pc", "halt", "bad"}
Instructions ==
    {[op |-> "rr", d |-> d, s |-> s, value |-> 0, address |-> "ram0"] :
        d \in Registers, s \in Registers}
    \cup {[op |-> "ri", d |-> d, s |-> 0, value |-> v, address |-> "ram0"] :
        d \in Registers, v \in Values}
    \cup {[op |-> "load", d |-> d, s |-> 0, value |-> 0, address |-> a] :
        d \in Registers, a \in Addresses}
    \cup {[op |-> "store", d |-> 0, s |-> s, value |-> 0, address |-> a] :
        s \in Registers, a \in Addresses}
    \cup {[op |-> "bad", d |-> 0, s |-> 0, value |-> 0, address |-> "bad"]}
DefaultInstruction == CHOOSE i \in Instructions : i.op = "bad"
VARIABLES pc, regs, ram, halted, fault, code, phase, instruction, previous, event
Arch == <<pc, regs, ram, halted, fault, code, phase, instruction>>
vars == <<pc, regs, ram, halted, fault, code, phase, instruction, previous, event>>
Snapshot == [pc |-> pc, regs |-> regs, ram |-> ram, halted |-> halted,
             fault |-> fault, code |-> code, phase |-> phase,
             instruction |-> instruction]
Advance == (pc + 1) % 4
RamIndex == IF instruction.address = "ram0" THEN 0 ELSE 1

Init == /\ pc = 0 /\ regs = [r \in Registers |-> 0]
        /\ ram = [r \in Registers |-> 0] /\ halted = FALSE /\ fault = FALSE
        /\ code = 0 /\ phase = "fetch" /\ instruction = DefaultInstruction
        /\ previous = Snapshot /\ event = "init"

Record(e) == /\ previous' = Snapshot /\ event' = e
Reset == /\ Record("reset") /\ pc' = 0 /\ regs' = [r \in Registers |-> 0]
         /\ halted' = FALSE /\ fault' = FALSE /\ code' = 0
         /\ phase' = "fetch" /\ instruction' = DefaultInstruction /\ UNCHANGED ram
Hold(e) == /\ Record(e) /\ UNCHANGED Arch
Fail(e) == /\ Record(e) /\ halted' = TRUE /\ fault' = TRUE
           /\ UNCHANGED <<pc, regs, ram, code, phase, instruction>>
Fetch == /\ ~halted /\ phase = "fetch"
         /\ \E i \in Instructions : instruction' = i
         /\ phase' = "exec" /\ Record("fetch")
         /\ UNCHANGED <<pc, regs, ram, halted, fault, code>>
Retire(e) == /\ Record(e) /\ phase' = "fetch" /\ instruction' = instruction
             /\ fault' = fault
Execute == /\ ~halted /\ phase = "exec"
           /\ CASE instruction.op = "bad" -> Fail("fault")
              [] instruction.op = "rr" ->
                  /\ Retire("rr") /\ pc' = Advance
                  /\ regs' = [regs EXCEPT ![instruction.d] = regs[instruction.s]]
                  /\ UNCHANGED <<ram, halted, code>>
              [] instruction.op = "ri" ->
                  /\ Retire("ri") /\ pc' = Advance
                  /\ regs' = [regs EXCEPT ![instruction.d] = instruction.value]
                  /\ UNCHANGED <<ram, halted, code>>
              [] instruction.address = "bad" -> Fail("fault")
              [] instruction.op = "load" /\ instruction.address = "halt" -> Fail("fault")
              [] instruction.op = "load" ->
                  /\ Retire("load") /\ pc' = Advance
                  /\ regs' = [regs EXCEPT ![instruction.d] =
                       IF instruction.address = "pc" THEN pc ELSE ram[RamIndex]]
                  /\ UNCHANGED <<ram, halted, code>>
              [] instruction.op = "store" /\ instruction.address = "pc" ->
                  /\ Retire("jump") /\ pc' = regs[instruction.s]
                  /\ UNCHANGED <<regs, ram, halted, code>>
              [] instruction.op = "store" /\ instruction.address = "halt" ->
                  /\ Retire("halt") /\ pc' = Advance /\ halted' = TRUE
                  /\ code' = regs[instruction.s] /\ UNCHANGED <<regs, ram>>
              [] OTHER ->
                  /\ Retire("store") /\ pc' = Advance
                  /\ ram' = [ram EXCEPT ![RamIndex] = regs[instruction.s]]
                  /\ UNCHANGED <<regs, halted, code>>
External == instruction.op \in {"load", "store"}
            /\ instruction.address \in {"ram0", "ram1"}
Next == Reset \/ (halted /\ Hold("stopped"))
        \/ (~halted /\ phase = "fetch" /\ (Hold("stall") \/ Fail("fault")))
        \/ Fetch \/ Execute
        \/ (~halted /\ phase = "exec" /\ External /\ (Hold("stall") \/ Fail("fault")))
Spec == Init /\ [][Next]_vars

TypeOK == /\ pc \in Values /\ regs \in [Registers -> Values]
          /\ ram \in [Registers -> Values] /\ code \in Values
          /\ halted \in BOOLEAN /\ fault \in BOOLEAN
          /\ phase \in {"fetch", "exec"} /\ instruction \in Instructions
FaultImpliesStop == fault => halted
PreciseFault == event = "fault" =>
    /\ halted /\ fault /\ pc = previous.pc /\ regs = previous.regs
    /\ ram = previous.ram /\ code = previous.code
StopSticky == event = "stopped" => Snapshot = previous
StallStable == event = "stall" => Snapshot = previous
ResetPriority == event = "reset" =>
    /\ pc = 0 /\ regs = [r \in Registers |-> 0] /\ ~halted /\ ~fault
    /\ code = 0 /\ ram = previous.ram /\ phase = "fetch"
RegisterFrame == event \in {"rr", "ri", "load"} =>
    \A r \in Registers \ {previous.instruction.d} : regs[r] = previous.regs[r]
MemoryFrame == event # "store" /\ event # "init" => ram = previous.ram
PCTransition == /\ (event \in {"rr", "ri", "load", "store", "halt"} =>
                         pc = (previous.pc + 1) % 4)
                /\ (event = "jump" => pc = previous.regs[previous.instruction.s])
MoveSemantics ==
    /\ (event = "rr" => regs[previous.instruction.d] = previous.regs[previous.instruction.s])
    /\ (event = "ri" => regs[previous.instruction.d] = previous.instruction.value)
    /\ (event = "load" => regs[previous.instruction.d] =
        IF previous.instruction.address = "pc" THEN previous.pc
        ELSE previous.ram[IF previous.instruction.address = "ram0" THEN 0 ELSE 1])
    /\ (event = "store" => ram = [previous.ram EXCEPT
        ![IF previous.instruction.address = "ram0" THEN 0 ELSE 1] =
            previous.regs[previous.instruction.s]])
    /\ (event = "halt" => halted /\ ~fault /\ code = previous.regs[previous.instruction.s])
=============================================================================
