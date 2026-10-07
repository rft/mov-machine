use mov_machine::compiler::{compile_ir, CompileOptions};
use mov_machine::{Machine, RunOutcome};

fn options(arguments: &[u32]) -> CompileOptions {
    CompileOptions {
        arguments: arguments.to_vec(),
        ram_words: 16_384,
        memory_bytes: 128,
        ..CompileOptions::default()
    }
}
fn machine(source: &str, arguments: &[u32]) -> Machine {
    let program = compile_ir(source, &options(arguments)).unwrap();
    assert!(program
        .image
        .instructions()
        .iter()
        .all(|i| i.header >> 26 <= 3));
    Machine::new(program.image, program.required_ram_words).unwrap()
}
fn execute(source: &str, arguments: &[u32]) -> u32 {
    let mut machine = machine(source, arguments);
    assert_eq!(
        machine.run(20_000_000),
        RunOutcome::Halted,
        "{:?}",
        machine.state()
    );
    machine.result().unwrap().unwrap()
}
fn binary(op: &str, bits: u8, a: u32, b: u32) -> u32 {
    execute(&format!("define i{bits} @main(i{bits} %a, i{bits} %b) {{\nentry:\n %r = {op} i{bits} %a, %b\n ret i{bits} %r\n}}"), &[a, b])
}
fn mask(bits: u8) -> u32 {
    if bits == 32 {
        u32::MAX
    } else {
        (1 << bits) - 1
    }
}
fn signed(value: u32, bits: u8) -> i64 {
    let value = (value & mask(bits)) as i64;
    if value & (1i64 << (bits - 1)) != 0 {
        value - (1i64 << bits)
    } else {
        value
    }
}

#[test]
fn variable_add_sub_mul_and_boolean_edges() {
    for bits in [1, 8, 16, 32] {
        let m = mask(bits);
        let pairs = if bits == 1 {
            vec![(0, 0), (0, 1), (1, 0), (1, 1)]
        } else {
            vec![
                (m, 1),
                (0, m),
                (m >> 1, (m >> 1) + 1),
                (m, m),
                (0x1234_5678 & m, 0x8765_4321 & m),
            ]
        };
        for (a, b) in pairs {
            for (op, expected) in [
                ("add", a.wrapping_add(b)),
                ("sub", a.wrapping_sub(b)),
                ("mul", a.wrapping_mul(b)),
                ("and", a & b),
                ("or", a | b),
                ("xor", a ^ b),
            ] {
                assert_eq!(binary(op, bits, a, b), expected & m, "{op} i{bits} {a} {b}");
            }
        }
    }
}

#[test]
fn variable_signed_and_unsigned_division_remainders() {
    for bits in [1, 8, 16, 32] {
        let m = mask(bits);
        for (a, b) in [
            (0, 1),
            (m, 1),
            (m, m),
            (1 << (bits - 1), 3 & m),
            (m.wrapping_sub(6), 3 & m),
            (23 & m, m.wrapping_sub(2)),
        ] {
            if b == 0 {
                continue;
            }
            assert_eq!(binary("udiv", bits, a, b), (a & m) / (b & m));
            assert_eq!(binary("urem", bits, a, b), (a & m) % (b & m));
            let sa = signed(a, bits);
            let sb = signed(b, bits);
            // LLVM's signed minimum/-1 is poison. Defined pairs use a wide
            // reference so that all supported integer widths are covered.
            if sa == -(1i64 << (bits - 1)) && sb == -1 {
                continue;
            }
            assert_eq!(binary("sdiv", bits, a, b), (sa / sb) as u32 & m);
            assert_eq!(binary("srem", bits, a, b), (sa % sb) as u32 & m);
        }
    }
}

#[test]
fn all_division_zero_paths_are_actual_machine_faults() {
    for op in ["udiv", "urem", "sdiv", "srem"] {
        let source = format!(
            "define i32 @main(i32 %a, i32 %b) {{\nentry:\n %r = {op} i32 %a, %b\n ret i32 %r\n}}"
        );
        let mut machine = machine(&source, &[99, 0]);
        assert_eq!(machine.run(20_000_000), RunOutcome::Fault);
        assert!(machine.state().fault);
        assert_eq!(machine.result().unwrap(), None);
    }
}

#[test]
fn variable_barrel_shifts_boundaries() {
    for bits in [1, 8, 16, 32] {
        let m = mask(bits);
        let a = (1 << (bits - 1)) | (0x53 & m);
        for distance in [0, bits as u32 / 2, bits as u32 - 1] {
            assert_eq!(
                binary("shl", bits, a, distance),
                a.wrapping_shl(distance) & m
            );
            assert_eq!(binary("lshr", bits, a, distance), a >> distance);
            assert_eq!(
                binary("ashr", bits, a, distance),
                (signed(a, bits) >> distance) as u32 & m
            );
        }
    }
    // An unselected overshift is LLVM poison, not a reason to fault.
    let source = "define i8 @main(i8 %a, i8 %b) {\nentry:\n %p = shl i8 %a, %b\n %r = select i1 false, i8 %p, i8 %a\n ret i8 %r\n}";
    assert_eq!(execute(source, &[19, 255]), 19);
}

#[test]
fn comparisons_cover_all_predicates_and_widths() {
    for bits in [1, 8, 16, 32] {
        let m = mask(bits);
        for (a, b) in [
            (m, 1),
            (0, m),
            (m, m),
            (1 << (bits - 1), (1 << (bits - 1)) - 1),
        ] {
            let sa = signed(a, bits);
            let sb = signed(b, bits);
            for (predicate, expected) in [
                ("eq", a == b),
                ("ne", a != b),
                ("ult", a < b),
                ("ule", a <= b),
                ("ugt", a > b),
                ("uge", a >= b),
                ("slt", sa < sb),
                ("sle", sa <= sb),
                ("sgt", sa > sb),
                ("sge", sa >= sb),
            ] {
                let source = format!("define i1 @main(i{bits} %a, i{bits} %b) {{\nentry:\n %r = icmp {predicate} i{bits} %a, %b\n ret i1 %r\n}}");
                assert_eq!(
                    execute(&source, &[a, b]),
                    u32::from(expected),
                    "{predicate} i{bits} {a} {b}"
                );
            }
        }
    }
}

#[test]
fn selects_and_casts_preserve_integer_and_pointer_bits() {
    let source = "define i32 @main(i8 %a, i1 %c) {\nentry:\n %s = sext i8 %a to i32\n %z = zext i8 %a to i32\n %t = trunc i32 %s to i16\n %back = sext i16 %t to i32\n %p = inttoptr i32 %back to ptr\n %alias = bitcast ptr %p to ptr\n %n = ptrtoint ptr %alias to i32\n %r = select i1 %c, i32 %n, i32 %z\n ret i32 %r\n}";
    assert_eq!(execute(source, &[0x81, 1]), 0xffff_ff81);
    assert_eq!(execute(source, &[0x81, 0]), 0x81);
}

#[test]
fn loop_phi_assignments_are_parallel() {
    let source = "define i32 @main(i32 %x, i32 %y, i32 %limit) {\nentry:\n br label %loop\nloop:\n %a = phi i32 [ %x, %entry ], [ %b, %body ]\n %b = phi i32 [ %y, %entry ], [ %a, %body ]\n %i = phi i32 [ 0, %entry ], [ %next, %body ]\n %done = icmp eq i32 %i, %limit\n br i1 %done, label %exit, label %body\nbody:\n %next = add i32 %i, 1\n br label %loop\nexit:\n %ten = mul i32 %a, 10\n %r = add i32 %ten, %b\n ret i32 %r\n}";
    assert_eq!(execute(source, &[2, 7, 3]), 72);
    assert_eq!(execute(source, &[2, 7, 4]), 27);
    assert_eq!(execute(source, &[2, 7, 0]), 27);
}

#[test]
fn nested_direct_calls_have_independent_frames_and_returns() {
    let source = "define i32 @bump(i32 %x) {\nentry:\n %p = alloca i32, align 4\n store i32 %x, ptr %p, align 4\n %v = load i32, ptr %p, align 4\n %r = add i32 %v, 1\n ret i32 %r\n}\ndefine i32 @twice(i32 %x) {\nentry:\n %a = call i32 @bump(i32 %x)\n %b = call i32 @bump(i32 %a)\n ret i32 %b\n}\ndefine i32 @main(i32 %x) {\nentry:\n %a = call i32 @twice(i32 %x)\n %b = call i32 @twice(i32 %a)\n ret i32 %b\n}";
    assert_eq!(execute(source, &[0xffff_fffe]), 2);
}

#[test]
fn global_initialization_variable_gep_and_byte_aliasing() {
    let source = "@data = global [4 x i32] [i32 11, i32 22, i32 33, i32 44], align 4\ndefine i32 @main(i32 %i, i32 %v) {\nentry:\n %p = getelementptr [4 x i32], ptr @data, i32 0, i32 %i\n %old = load i32, ptr %p, align 4\n store i32 %v, ptr %p, align 4\n %byte = getelementptr i8, ptr %p, i32 1\n %b = load i8, ptr %byte, align 1\n %z = zext i8 %b to i32\n %r = add i32 %old, %z\n ret i32 %r\n}";
    assert_eq!(execute(source, &[2, 0x1234_5600]), 33 + 0x56);
    let mut machine = machine(source, &[2, 0x1234_5600]);
    assert_eq!(machine.run(20_000_000), RunOutcome::Halted);
    assert_eq!(machine.result().unwrap(), Some(119));
    machine.ram_mut().fill(0xdead_beef);
    machine.reset();
    assert_eq!(machine.run(20_000_000), RunOutcome::Halted);
    assert_eq!(machine.result().unwrap(), Some(119));
}

#[test]
fn nested_array_alloca_gep_uses_logical_bytes_and_signed_indices() {
    let source = "define i32 @main(i32 %i, i16 %v) {\nentry:\n %a = alloca [2 x [3 x i16]], align 2\n %p = getelementptr [2 x [3 x i16]], ptr %a, i32 0, i32 1, i32 %i\n store i16 %v, ptr %p, align 2\n %q = getelementptr i16, ptr %p, i8 -1\n store i16 7, ptr %q, align 2\n %x = load i16, ptr %p, align 2\n %y = load i16, ptr %q, align 2\n %sx = zext i16 %x to i32\n %sy = zext i16 %y to i32\n %r = add i32 %sx, %sy\n ret i32 %r\n}";
    assert_eq!(execute(source, &[2, 0xabcd]), 0xabcd + 7);
}

#[test]
fn null_and_multibyte_out_of_bounds_accesses_fault() {
    let source =
        "define i32 @main(ptr %p) {\nentry:\n %r = load i32, ptr %p, align 1\n ret i32 %r\n}";
    for pointer in [0, 125, 128, 0xffff_ffff] {
        let mut machine = machine(source, &[pointer]);
        assert_eq!(
            machine.run(20_000_000),
            RunOutcome::Fault,
            "pointer {pointer}"
        );
    }
    let store = "define void @main(ptr %p) {\nentry:\n store i32 99, ptr %p, align 1\n ret void\n}";
    for pointer in [0, 127, 128] {
        let mut machine = machine(store, &[pointer]);
        assert_eq!(machine.run(20_000_000), RunOutcome::Fault);
    }
}

#[test]
fn void_call_and_conditional_callee_return() {
    let source = "@g = global i8 5\ndefine void @put(i8 %v) {\nentry:\n store i8 %v, ptr @g\n ret void\n}\ndefine i8 @choose(i1 %c, i8 %a, i8 %b) {\nentry:\n br i1 %c, label %yes, label %no\nyes:\n ret i8 %a\nno:\n ret i8 %b\n}\ndefine i8 @main(i1 %c) {\nentry:\n call void @put(i8 17)\n %v = load i8, ptr @g\n %r = call i8 @choose(i1 %c, i8 %v, i8 29)\n ret i8 %r\n}";
    assert_eq!(execute(source, &[1]), 17);
    assert_eq!(execute(source, &[0]), 29);
    assert_eq!(
        execute("define void @main() {\nentry:\n ret void\n}", &[]),
        0
    );
}

#[test]
fn pointer_values_round_trip_through_source_memory() {
    let source = "@g = global i32 31, align 4\ndefine i32 @main(i32 %v) {\nentry:\n %slot = alloca ptr, align 4\n store ptr @g, ptr %slot, align 4\n %p = load ptr, ptr %slot, align 4\n store i32 %v, ptr %p, align 4\n %r = load i32, ptr @g, align 4\n ret i32 %r\n}";
    assert_eq!(execute(source, &[0x8765_4321]), 0x8765_4321);
}

#[test]
fn final_valid_word_and_unaligned_word_accesses() {
    let source = "define i32 @main(ptr %p, i32 %v) {\nentry:\n store i32 %v, ptr %p, align 1\n %r = load i32, ptr %p, align 1\n ret i32 %r\n}";
    assert_eq!(execute(source, &[124, 0xdead_beef]), 0xdead_beef);
    assert_eq!(execute(source, &[1, 0x0102_0304]), 0x0102_0304);
}

#[test]
fn constant_arithmetic_and_boolean_simplification_preserve_semantics() {
    for (op, a, b, expected) in [
        ("add", 255u32, 2u32, 1u32),
        ("sub", 0, 1, 255),
        ("mul", 17, 19, 67),
        ("sdiv", 249, 3, 254),
        ("srem", 249, 3, 255),
        ("udiv", 255, 3, 85),
        ("urem", 255, 7, 3),
        ("ashr", 128, 7, 255),
        ("lshr", 128, 7, 1),
        ("shl", 17, 3, 136),
    ] {
        let source =
            format!("define i8 @main() {{\nentry:\n %r = {op} i8 {a}, {b}\n ret i8 %r\n}}");
        assert_eq!(execute(&source, &[]), expected);
    }
    let source = "define i8 @main(i8 %x) {\nentry:\n %a = xor i8 %x, 255\n %b = and i8 %a, 255\n %r = or i8 %b, 0\n ret i8 %r\n}";
    assert_eq!(execute(source, &[0x53]), 0xac);
    let zero = "define i8 @main() {\nentry:\n %r = udiv i8 19, 0\n ret i8 %r\n}";
    let mut machine = machine(zero, &[]);
    assert_eq!(machine.run(20_000_000), RunOutcome::Fault);
}

#[test]
fn rejects_recursion_externals_unsupported_ir_and_resources() {
    let recursive = "define i32 @main() {\nentry:\n %r = call i32 @main()\n ret i32 %r\n}";
    assert!(compile_ir(recursive, &options(&[])).is_err());
    let external = "declare i32 @unknown()\ndefine i32 @main() {\nentry:\n %r = call i32 @unknown()\n ret i32 %r\n}";
    assert!(compile_ir(external, &options(&[])).is_err());
    for source in [
        "define i32 @main() {\nentry:\n %r = freeze i32 1\n ret i32 %r\n}",
        "define i32 @main() {\nentry:\n ret i32 poison\n}",
        "define i64 @main() {\nentry:\n ret i64 0\n}",
    ] {
        assert!(compile_ir(source, &options(&[])).is_err());
    }
    let source = "define i32 @main(i32 %a) {\nentry:\n %r = add i32 %a, 1\n ret i32 %r\n}";
    let mut small = options(&[7]);
    small.ram_words = 2;
    assert!(compile_ir(source, &small).is_err());
    let mut invalid = options(&[7]);
    invalid.memory_bytes = 3;
    assert!(compile_ir(source, &invalid).is_err());
    assert!(compile_ir(source, &options(&[])).is_err());
    let huge = "define void @main() {\nentry:\n %p = alloca [128 x i8]\n ret void\n}";
    assert!(compile_ir(huge, &options(&[])).is_err());
}
