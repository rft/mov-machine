use mov_machine::*;
fn machine(source: &str, words: usize) -> Machine {
    Machine::new(assemble(source).unwrap(), words).unwrap()
}
#[test]
fn mixed_assembly_labels_and_image_roundtrip() {
    let image = assemble(".text\n.globl start\n.type start, @function\n.p2align 2\nstart: mov.ri r0, #end // forward\nmov.rr r1, r0\nmov.mr [r2 + 4], r1\nmov.rm r3, [r2 + 4]\nend: mov r2, -2147483648\n.size start, .-start").unwrap();
    assert_eq!(image.instructions()[0].extension, 4);
    assert_eq!(image.instructions()[2], Instruction::new(3, 2, 1, 4));
    assert_eq!(image.instructions()[3], Instruction::new(2, 3, 2, 4));
    assert_eq!(Image::from_bytes(&image.to_bytes()).unwrap(), image);
    assert!(Image::from_bytes(&[0; 7]).is_err());
}
#[test]
fn rejects_pseudos_directives_and_bad_operands() {
    for source in [
        "mov_add r0, r1",
        "mov.neg r0, r1",
        "add r0, r1",
        ".word 0",
        ".section .text",
        ".align 4",
        ".unknown",
        "mov.rr r0, 4",
        "mov.rm [r0], r1",
        "mov r4, r1",
        "mov r0, r1, r2",
        "mov r0, #4294967296",
        "mov r0, #-2147483649",
        "mov [r0 ++ 4], r1",
        "x: mov r0, 0\nx: mov r0, 1",
        "mov r0, missing",
    ] {
        assert!(assemble(source).is_err(), "{source}");
    }
    assert_eq!(
        assemble("mov r0, 0xffffffff\nmov r1, -1\nmov r2, -2147483648")
            .unwrap()
            .instructions()[1]
            .extension,
        u32::MAX
    );
}
#[test]
fn malformed_decode_has_no_architectural_writes() {
    for bad in [
        Instruction {
            header: 1,
            extension: 0,
        },
        Instruction::new(4, 0, 0, 0),
        Instruction::new(0, 0, 1, 1),
        Instruction::new(1, 0, 1, 123),
    ] {
        let mut m = Machine::new(Image::new(vec![Instruction::new(1, 0, 0, 42), bad]), 1).unwrap();
        m.step();
        let before = m.state();
        let step = m.step();
        assert!(!step.retired);
        assert_eq!(step.memory_write, None);
        assert_eq!(m.state().registers, before.registers);
        assert_eq!(m.state().pc, before.pc);
        assert_eq!(m.ram(), &[0]);
        assert!(m.state().fault && m.state().halted);
        assert!(!m.step().executed);
    }
}
#[test]
fn load_register_alias_and_wrapping_address() {
    let mut m = machine("mov r0, 4\nmov r1, 99\nmov [r0], r1\nmov r0, [r0]\nmov r2, -4\nmov [r2 + 4], r0\nmov r3, [r2 + 4]", 2);
    for _ in 0..7 {
        assert!(m.step().retired);
    }
    assert_eq!(m.state().registers, [99, 99, u32::MAX - 3, 99]);
    assert_eq!(m.ram(), &[99, 99]);
}
#[test]
fn ram_limits_misalignment_and_halt_read_fault() {
    for source in [
        "mov r0, [r1 + 4]",
        "mov [r1 + 4], r0",
        "mov r0, [r1 + 1]",
        "mov [r1 + 1], r0",
        "mov r0, [r1 - 8]",
    ] {
        let mut m = machine(source, 1);
        assert!(!m.step().retired);
        assert_eq!(m.state().pc, 0);
        assert_eq!(m.state().registers, [0; 4]);
        assert_eq!(m.ram(), &[0]);
    }
    let mut empty = machine("mov r0, [r1]", 0);
    assert!(matches!(empty.run(1), RunOutcome::Fault));
}
#[test]
fn pc_read_jump_halt_and_reset() {
    let mut m = machine("mov r0, -4\nmov r1, [r0]\nmov r2, target\nmov [r0], r2\nmov r1, 999\ntarget: mov r3, -8\nmov [r3], r1", 1);
    m.ram_mut()[0] = 77;
    assert_eq!(m.run(20), RunOutcome::Halted);
    assert_eq!(m.state().registers[1], 1);
    assert_eq!(m.state().halt_code, 1);
    assert_eq!(m.state().pc, 7);
    assert_eq!(m.state().steps, 6);
    let stopped = m.state();
    assert!(!m.step().executed);
    assert_eq!(m.state(), stopped);
    m.reset();
    assert_eq!(m.state().registers, [0; 4]);
    assert_eq!(m.state().pc, 0);
    assert!(!m.state().halted);
    assert_eq!(m.ram(), &[77]);
}
#[test]
fn out_of_image_fault_and_bounded_loop() {
    let mut empty = Machine::new(Image::new(vec![]), 0).unwrap();
    assert_eq!(empty.run(0), RunOutcome::BudgetExhausted);
    assert_eq!(empty.run(1), RunOutcome::Fault);
    assert_eq!(empty.state().steps, 1);
    let mut looping = machine("mov r0, -4\nmov [r0], r1", 0);
    assert_eq!(looping.run(12), RunOutcome::BudgetExhausted);
    assert_eq!(looping.state().steps, 12);
    assert!(!looping.state().halted);
    let mut jump = machine("mov r0, -4\nmov r1, -1\nmov [r0], r1", 0);
    for _ in 0..3 {
        jump.step();
    }
    assert_eq!(jump.state().pc, u32::MAX);
    assert!(!jump.step().retired);
    assert_eq!(jump.state().pc, u32::MAX);
}
#[test]
fn trace_reports_successful_stores_only() {
    let mut m = machine("mov r0, 7\nmov [r1], r0\nmov r2, -8\nmov [r2], r0", 1);
    assert_eq!(m.step().memory_write, None);
    assert_eq!(
        m.step().memory_write,
        Some(MemoryWrite {
            address: 0,
            value: 7
        })
    );
    m.step();
    let halt = m.step();
    assert_eq!(
        halt.memory_write,
        Some(MemoryWrite {
            address: HALT_ADDRESS,
            value: 7
        })
    );
    assert!(halt.retired);
    assert!(halt
        .to_json()
        .contains("\"memory_write\":{\"address\":4294967288,\"value\":7}"));
}
