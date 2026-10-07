use mov_machine::{assemble, Image, Instruction, Machine, RunOutcome};

#[test]
fn serialized_executable_runs_with_native_halt_abi() {
    let source = assemble("mov r0, -8\nmov r1, 42\nmov [r0], r1").unwrap();
    let image = Image::from_bytes(&source.to_bytes()).unwrap();
    let mut machine = Machine::new(image, 0).unwrap();
    assert_eq!(machine.run(3), RunOutcome::Halted);
    assert_eq!(machine.result().unwrap(), Some(42));
}

#[test]
fn rejects_truncated_counts_versions_reserved_and_invalid_return_regions() {
    let bytes = assemble("mov r0, 42").unwrap().to_bytes();
    for end in [0, 7, 31, bytes.len() - 1] {
        assert!(Image::from_bytes(&bytes[..end]).is_err());
    }
    for (offset, value) in [(4, 2), (8, 2), (20, 33), (24, 1)] {
        let mut malformed = bytes.clone();
        malformed[offset] = value;
        assert!(Image::from_bytes(&malformed).is_err());
    }
    assert!(Image::compiled(vec![], 8, 4, 32).is_err());
    assert!(Image::compiled(vec![], 8, 3, 1).is_err());
    assert!(Image::compiled(vec![], 8, 4, 0).is_err());
}

#[test]
fn encoded_result_requires_capacity_and_boolean_words() {
    // Target executes only MOV. The host decodes a little-endian Boolean vector.
    let value = 0x8000_00a5u32;
    let mut code = vec![Instruction::new(1, 3, 0, 0)];
    for bit in 0..32 {
        code.push(Instruction::new(1, 0, 0, ((value >> bit) & 1) * 4));
        code.push(Instruction::new(3, 3, 0, 8 + bit * 4));
    }
    code.extend([
        Instruction::new(1, 0, 0, 0xffff_fff8),
        Instruction::new(1, 1, 0, 8),
        Instruction::new(3, 0, 1, 0),
    ]);
    let image = Image::compiled(code, 34, 8, 32).unwrap();
    assert!(Machine::new(image.clone(), 33).is_err());
    let mut machine = Machine::new(Image::from_bytes(&image.to_bytes()).unwrap(), 34).unwrap();
    assert_eq!(machine.result().unwrap(), None);
    assert_eq!(machine.run(100), RunOutcome::Halted);
    assert_eq!(machine.state().halt_code, 8);
    assert_eq!(machine.result().unwrap(), Some(value));
    machine.ram_mut()[2] = 1;
    assert!(machine.result().is_err());
    machine.reset();
    assert_eq!(machine.result().unwrap(), None);
}
