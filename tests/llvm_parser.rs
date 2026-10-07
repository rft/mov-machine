use mov_machine::llvm::{parse_module, BinaryOp, CastOp, Instruction, Operand, Predicate, Type};

fn rejected(source: &str, line: usize) {
    let error = parse_module(source).expect_err("invalid executable LLVM must be rejected");
    assert_eq!(error.line, line, "{error}");
}

#[test]
fn clang_style_numeric_locals_attributes_and_metadata() {
    let module = parse_module(r#"
source_filename = "sum.c"
target datalayout = "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-f64:32:64-f80:32-n8:16:32-S128"
target triple = "i386-none-unknown-elf"
; Function Attrs: mustprogress nofree norecurse nosync nounwind willreturn memory(none)
define dso_local zeroext i8 @sum(i8 noundef zeroext %0,
                               i8 noundef zeroext %1) local_unnamed_addr #0 !dbg !5 {
  %3 = add i8 %1, %0, !dbg !6
  ret i8 %3, !dbg !7
}
attributes #0 = { mustprogress nofree norecurse nosync nounwind willreturn memory(none) "target-cpu"="pentium4" }
!llvm.module.flags = !{!0}
!0 = !{i32 1, !"wchar_size", i32 4}
!5 = distinct !DISubprogram(name: "sum", scope: !8, line: 1)
!6 = !DILocation(line: 2, column: 12, scope: !5)
!7 = !DILocation(line: 2, column: 5, scope: !5)
!8 = !DIFile(filename: "sum.c", directory: "/tmp")
"#).unwrap();
    let function = &module.functions[0];
    assert_eq!(
        function.parameters,
        vec![("0".into(), Type::Int(8)), ("1".into(), Type::Int(8))]
    );
    assert_eq!(function.blocks[0].label, "2");
    assert_eq!(
        function.blocks[0].instructions[0],
        Instruction::Binary {
            dst: "3".into(),
            op: BinaryOp::Add,
            ty: Type::Int(8),
            lhs: Operand::Local("1".into()),
            rhs: Operand::Local("0".into()),
        }
    );
}

#[test]
fn multiline_grammar_not_line_patterns_and_signed_literals() {
    let module = parse_module(
        r#"
define i16 @calculate(
    i8 %x,
    i1 %choose
) {
entry:
  %a = add
    nsw i8
    %x,
    -1 ; bits are 255
  %wide = sext i8 %a
    to i16
  %choice = select
    i1 %choose,
    i16 %wide,
    i16 -2
  ret
    i16 %choice
}
"#,
    )
    .unwrap();
    let instructions = &module.functions[0].blocks[0].instructions;
    assert!(matches!(
        &instructions[0],
        Instruction::Binary {
            rhs: Operand::Constant(255),
            ..
        }
    ));
    assert!(matches!(
        &instructions[1],
        Instruction::Cast {
            op: CastOp::SExt,
            from: Type::Int(8),
            to: Type::Int(16),
            ..
        }
    ));
    assert!(matches!(
        &instructions[2],
        Instruction::Select {
            on_false: Operand::Constant(65534),
            ..
        }
    ));
}

#[test]
fn loop_phis_are_checked_on_predecessor_edges() {
    let module = parse_module(
        r#"
define i32 @loop(i32 %limit) {
entry:
  br label %body
body:
  %i = phi i32 [ 0, %entry ],
               [ %next, %back ]
  %carry = phi i32 [ 9, %entry ], [ %i, %back ]
  %keep = icmp ult i32 %i, %limit
  br i1 %keep, label %back, label %exit
back:
  %next = add i32 %i, 1
  br label %body
exit:
  ret i32 %carry
}
"#,
    )
    .unwrap();
    assert_eq!(module.functions[0].blocks.len(), 4);
    assert!(
        matches!(&module.functions[0].blocks[1].instructions[0], Instruction::Phi { incoming, .. } if incoming.len() == 2)
    );
    assert!(matches!(
        &module.functions[0].blocks[1].instructions[2],
        Instruction::ICmp {
            predicate: Predicate::Ult,
            ..
        }
    ));
}

#[test]
fn globals_arrays_strings_and_logical_gep_types() {
    let module = parse_module(
        r#"
@values = private unnamed_addr constant [3 x i16] [i16 -1, i16 2, i16 513], align 2
@text = internal constant [3 x i8] c"A\00\FF", align 1
@matrix = global [2 x [2 x i8]] [[2 x i8] [i8 1, i8 2], [2 x i8] zeroinitializer]
@nil = global ptr null, align 4

define i16 @read(i32 %index) {
entry:
  %local = alloca [2 x i16], i32 2, align 4
  %p = getelementptr inbounds [3 x i16], ptr @values, i32 0, i32 %index
  %x = load i16, ptr %p, align 2
  store i16 %x, ptr %local, align 2
  ret i16 %x
}
"#,
    )
    .unwrap();
    assert_eq!(module.globals[0].initializer, vec![255, 255, 2, 0, 1, 2]);
    assert_eq!(module.globals[1].initializer, vec![65, 0, 255]);
    assert_eq!(module.globals[2].initializer, vec![1, 2, 0, 0]);
    assert_eq!(module.globals[3].initializer, vec![0; 4]);
    assert_eq!(module.globals[0].ty.size_bytes().unwrap(), 6);
    assert!(matches!(
        &module.functions[0].blocks[0].instructions[0],
        Instruction::Alloca {
            count: 2,
            align: 4,
            ..
        }
    ));
    assert!(
        matches!(&module.functions[0].blocks[0].instructions[1], Instruction::Gep { indices, .. } if indices.len() == 2)
    );
}

#[test]
fn calls_forward_definitions_void_returns_and_parameter_attributes() {
    let module = parse_module(
        r#"
declare void @external(ptr nocapture noundef)
define i8 @main(i8 %x, ptr %p) {
entry:
  call void @external(ptr noundef nonnull align 4 dereferenceable(4) %p)
  %result = tail call zeroext i8 @helper(i8 noundef zeroext %x) #0
  ret i8 %result
}
define internal zeroext i8 @helper(i8 noundef zeroext %value) {
entry:
  ret i8 %value
}
attributes #0 = { nounwind }
"#,
    )
    .unwrap();
    assert_eq!(module.functions.len(), 2);
    assert!(matches!(
        &module.functions[0].blocks[0].instructions[0],
        Instruction::Call {
            dst: None,
            return_type: None,
            ..
        }
    ));
    assert!(
        matches!(&module.functions[0].blocks[0].instructions[1], Instruction::Call { dst: Some(dst), function, args, .. } if dst == "result" && function == "helper" && args[0].0 == Type::Int(8))
    );
}

#[test]
fn all_integer_operations_and_supported_casts() {
    let source = r#"
define i32 @ops(i32 %x, i32 %y, ptr %p) {
entry:
  %a = add nuw nsw i32 %x, %y
  %b = sub i32 %a, %x
  %c = mul i32 %b, %y
  %d = udiv exact i32 %c, 2
  %e = sdiv i32 %d, -1
  %f = urem i32 %e, 7
  %g = srem i32 %f, 3
  %h = and i32 %g, 255
  %i = or disjoint i32 %h, 256
  %j = xor i32 %i, %x
  %k = shl nuw i32 %j, 1
  %l = lshr exact i32 %k, 1
  %m = ashr i32 %l, 2
  %small = trunc nuw i32 %m to i8
  %wide = zext nneg i8 %small to i16
  %signed = sext i16 %wide to i32
  %address = ptrtoint ptr %p to i32
  %pointer = inttoptr i32 %address to ptr
  %same = bitcast ptr %pointer to ptr
  %equal = icmp eq ptr %same, null
  %answer = select i1 %equal, i32 %signed, i32 %address
  ret i32 %answer
}
"#;
    let module = parse_module(source).unwrap();
    let instructions = &module.functions[0].blocks[0].instructions;
    let operations: Vec<_> = instructions
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::Binary { op, .. } => Some(*op),
            _ => None,
        })
        .collect();
    assert_eq!(
        operations,
        vec![
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::UDiv,
            BinaryOp::SDiv,
            BinaryOp::URem,
            BinaryOp::SRem,
            BinaryOp::And,
            BinaryOp::Or,
            BinaryOp::Xor,
            BinaryOp::Shl,
            BinaryOp::LShr,
            BinaryOp::AShr
        ]
    );
}

#[test]
fn quoted_names_comments_and_explicit_address_space_zero() {
    let module = parse_module(
        r#"
@"a; b" = global i8 7 ; the semicolon inside the name is not a comment
define i8 @"read byte"(ptr addrspace(0) %"source pointer") {
"entry block":
  %"loaded byte" = load i8, ptr addrspace(0) %"source pointer"
  ret i8 %"loaded byte"
}
"#,
    )
    .unwrap();
    assert_eq!(module.globals[0].name, "a; b");
    assert_eq!(module.functions[0].name, "read byte");
    assert_eq!(module.functions[0].blocks[0].label, "entry block");
}

#[test]
fn unnamed_parameters_blocks_and_discarded_call_results_use_llvm_slots() {
    let module = parse_module(
        r#"
declare i32 @helper(i32)
define i32 @main(i32) {
  call i32 @helper(i32 %0)
  %3 = add i32 %2, -1
  ret i32 %3
}
"#,
    )
    .unwrap();
    let main = &module.functions[0];
    assert_eq!(main.parameters[0].0, "0");
    assert_eq!(main.blocks[0].label, "1");
    assert!(
        matches!(&main.blocks[0].instructions[0], Instruction::Call { dst: Some(dst), .. } if dst == "2")
    );
}

#[test]
fn duplicate_branch_edges_require_matching_duplicate_phi_inputs() {
    let module = parse_module(
        r#"
define i8 @both(i1 %c) {
entry:
  br i1 %c, label %join, label %join
join:
  %x = phi i8 [ 7, %entry ], [ 7, %entry ]
  ret i8 %x
}
"#,
    )
    .unwrap();
    assert!(
        matches!(&module.functions[0].blocks[1].instructions[0], Instruction::Phi { incoming, .. } if incoming.len() == 2)
    );
    rejected("define i8 @both(i1 %c) {\nentry:\n br i1 %c, label %join, label %join\njoin:\n %x = phi i8 [7, %entry], [8, %entry]\n ret i8 %x\n}", 5);
}

#[test]
fn unsupported_executable_constructs_are_never_skipped() {
    for instruction in [
        "%x = fadd float 1.0, 2.0",
        "switch i32 0, label %entry [ ]",
        "%x = freeze i8 1",
        "%x = load volatile i8, ptr null",
        "store atomic i8 1, ptr null seq_cst, align 1",
        "%x = extractvalue [1 x i8] zeroinitializer, 0",
        "call void asm sideeffect \"\", \"\"()",
    ] {
        rejected(
            &format!("define void @main() {{\nentry:\n {instruction}\n ret void\n}}"),
            3,
        );
    }
    rejected(
        "define i8 @main() {\nentry:\n ret i8 ptrtoint (ptr null to i8)\n}",
        3,
    );
    rejected("@p = global ptr getelementptr (i8, ptr null, i32 1)", 1);
    rejected("@p = global ptr @other", 1);
    rejected("@x = global i8 0\nsurprise arbitrary top-level text", 2);
}

#[test]
fn explicit_undef_poison_and_unsupported_types_are_errors() {
    for value in ["undef", "poison", "null", "true"] {
        rejected(
            &format!("define i8 @main() {{\nentry:\n ret i8 {value}\n}}"),
            3,
        );
    }
    rejected("@x = global [1 x i8] [i8 poison]", 1);
    rejected("define i64 @main() { ret i64 1 }", 1);
    rejected("define i8 @main(ptr addrspace(1) %p) { ret i8 1 }", 1);
    rejected(
        "define void @main() {\nentry:\n %x = load [2 x i8], ptr null\n ret void\n}",
        3,
    );
}

#[test]
fn source_pointer_layout_and_endianness_are_validated() {
    rejected("target datalayout = \"E-p:32:32\"", 1);
    rejected("target datalayout = \"e-p:64:64\"", 1);
    rejected("target datalayout = \"e-p0:64:64\"", 1);
    rejected("target datalayout = \"e-p:32\"", 1);
    rejected("target datalayout = \"e-p:32:64\"", 1);
    rejected("target datalayout = \"e-p:32:32:32:16\"", 1);
    assert!(
        parse_module("target datalayout = \"e-p0:32:32-p272:64:64\"")
            .unwrap()
            .functions
            .is_empty()
    );
}

#[test]
fn wrong_operand_types_and_invalid_casts_are_errors_at_the_use() {
    rejected(
        "define i32 @f(i8 %a) {\nentry:\n %x = add i32 %a, 1\n ret i32 %x\n}",
        3,
    );
    rejected(
        "define i8 @f(ptr %p) {\nentry:\n %x = add ptr %p, null\n ret i8 0\n}",
        3,
    );
    rejected(
        "define i8 @f(i8 %x) {\nentry:\n %v = zext i8 %x to i8\n ret i8 %v\n}",
        3,
    );
    rejected(
        "define i8 @f(i16 %x) {\nentry:\n %v = bitcast i16 %x to i8\n ret i8 %v\n}",
        3,
    );
    rejected(
        "define i8 @f(i8 %x) {\nentry:\n %v = inttoptr ptr null to ptr\n ret i8 %x\n}",
        3,
    );
    rejected(
        "define i8 @f(i8 %x) {\nentry:\n %v = select i8 %x, i8 1, i8 2\n ret i8 %v\n}",
        3,
    );
    rejected(
        "define i8 @f(i1 %c) {\nentry:\n %v = select i1 %c, i8 1, i16 2\n ret i8 %v\n}",
        3,
    );
    rejected("define i32 @f() {\nentry:\n ret i8 1\n}", 3);
    rejected(
        "define void @f() {\nentry:\n br i8 1, label %a, label %b\na:\n ret void\nb:\n ret void\n}",
        3,
    );
}

#[test]
fn definitions_names_and_references_are_validated() {
    rejected("define i8 @f(i8 %x, i8 %x) {\nentry:\n ret i8 %x\n}", 1);
    rejected(
        "define i8 @f() {\nentry:\n %x = add i8 1, 2\n %x = sub i8 2, 1\n ret i8 %x\n}",
        4,
    );
    rejected(
        "define void @f() {\nentry:\n ret void\nentry:\n ret void\n}",
        4,
    );
    rejected("define i8 @f() {\nentry:\n ret i8 %missing\n}", 3);
    rejected(
        "define i8 @f() {\nentry:\n %x = load i8, ptr @missing\n ret i8 %x\n}",
        3,
    );
    rejected("@x = global i8 0\n@x = global i8 1", 2);
    rejected(
        "define void @f() { ret void }\ndefine void @f() { ret void }",
        2,
    );
    rejected("define i8 @f() {\n %9 = add i8 1, 2\n ret i8 %9\n}", 2);
}

#[test]
fn cfg_requires_known_targets_single_terminators_and_entry_without_backedges() {
    rejected("define void @f() {\nentry:\n br label %missing\n}", 3);
    rejected("define void @f() {\nentry:\n br label %entry\n}", 2);
    rejected("define void @f() {\nentry:\n %x = add i8 1, 2\n}", 3);
    rejected(
        "define void @f() {\nentry:\n ret void\n %x = add i8 1, 2\n}",
        3,
    );
    rejected("define void @f() {\nentry:\n}\n", 2);
}

#[test]
fn dominance_is_not_just_defined_somewhere() {
    rejected(
        "define i8 @f() {\nentry:\n %x = add i8 %later, 1\n %later = add i8 1, 2\n ret i8 %x\n}",
        3,
    );
    rejected("define i8 @f(i1 %c) {\nentry:\n br i1 %c, label %left, label %right\nleft:\n %x = add i8 1, 2\n br label %join\nright:\n br label %join\njoin:\n ret i8 %x\n}", 10);
    rejected("define i8 @f(i1 %c) {\nentry:\n br i1 %c, label %left, label %right\nleft:\n %x = add i8 1, 2\n br label %join\nright:\n br label %join\njoin:\n %p = phi i8 [1, %left], [%x, %right]\n ret i8 %p\n}", 10);
}

#[test]
fn phi_placement_types_and_edge_coverage_are_validated() {
    rejected("define i8 @f() {\nentry:\n br label %join\njoin:\n %x = phi i8 [1, %entry], [2, %entry]\n ret i8 %x\n}", 5);
    rejected("define i8 @f(i1 %c) {\nentry:\n br i1 %c, label %left, label %join\nleft:\n br label %join\njoin:\n %x = phi i8 [1, %entry]\n ret i8 %x\n}", 7);
    rejected("define i8 @f() {\nentry:\n br label %join\njoin:\n %y = add i8 1, 2\n %x = phi i8 [1, %entry]\n ret i8 %x\n}", 6);
    rejected("define i8 @f(i16 %a) {\nentry:\n br label %join\njoin:\n %x = phi i8 [%a, %entry]\n ret i8 %x\n}", 5);
    rejected(
        "define i8 @f() {\nentry:\n br label %join\njoin:\n %x = phi i8 [1, %join]\n ret i8 %x\n}",
        5,
    );
}

#[test]
fn static_allocas_and_gep_indices_are_checked() {
    rejected(
        "define void @f(i32 %n) {\nentry:\n %x = alloca i8, i32 %n\n ret void\n}",
        3,
    );
    rejected(
        "define void @f() {\nentry:\n br label %other\nother:\n %x = alloca i8\n ret void\n}",
        5,
    );
    rejected(
        "define void @f() {\nentry:\n %x = alloca i8, align 3\n ret void\n}",
        3,
    );
    rejected("define ptr @f(ptr %p) {\nentry:\n %x = getelementptr i8, ptr %p, i32 0, i32 1\n ret ptr %x\n}", 3);
    rejected("define ptr @f(ptr %p) {\nentry:\n %x = getelementptr [2 x i8], ptr %p, ptr null\n ret ptr %x\n}", 3);
    rejected("@x = global [2 x i8] [i8 1]", 1);
    rejected("@x = global [1 x i8] [i16 1]", 1);
    rejected("@x = global [2 x i8] c\"x\"", 1);
}

#[test]
fn call_signatures_and_nonrepresentable_abis_are_rejected() {
    rejected("declare i8 @callee(i16)\ndefine i8 @f() {\nentry:\n %x = call i8 @callee(i8 1)\n ret i8 %x\n}", 4);
    rejected(
        "declare void @callee()\ndefine i8 @f() {\nentry:\n %x = call i8 @callee()\n ret i8 %x\n}",
        4,
    );
    rejected(
        "define void @f() {\nentry:\n %x = call void @unknown()\n ret void\n}",
        3,
    );
    rejected(
        "define i8 @f(ptr %fn) {\nentry:\n %x = call i8 %fn()\n ret i8 %x\n}",
        3,
    );
    rejected("define void @f(ptr byval(i8) %p) { ret void }", 1);
    rejected("define void @f(i8 %x, ...) { ret void }", 1);
    rejected(
        "declare void @f(...)\ndefine void @g() {\nentry:\n call void @f()\n ret void\n}",
        4,
    );
}

#[test]
fn malformed_input_does_not_become_an_executable_module() {
    rejected(
        "define i8 @f() {\nentry:\n %x = add i8 1 2\n ret i8 %x\n}",
        3,
    );
    rejected(
        "define i8 @f() {\nentry:\n %x = and nsw i8 1, 2\n ret i8 %x\n}",
        3,
    );
    rejected("define void @f() {\nentry:\n ret void, !dbg\n}", 4);
    rejected("source_filename = \"unterminated", 1);
    rejected("attributes #0 = { nounwind ]", 1);
    rejected("!0 = !{i32 1", 1);
    rejected("define void @f() {\nentry:\n ret void", 3);
}

#[test]
fn every_integer_predicate_preserves_its_signedness() {
    for (spelling, expected) in [
        ("eq", Predicate::Eq),
        ("ne", Predicate::Ne),
        ("ult", Predicate::Ult),
        ("ule", Predicate::Ule),
        ("ugt", Predicate::Ugt),
        ("uge", Predicate::Uge),
        ("slt", Predicate::Slt),
        ("sle", Predicate::Sle),
        ("sgt", Predicate::Sgt),
        ("sge", Predicate::Sge),
    ] {
        let module = parse_module(&format!(
            "define i1 @compare(i16 %a, i16 %b) {{\nentry:\n %c = icmp {spelling} i16 %a, %b\n ret i1 %c\n}}"
        )).unwrap();
        assert!(matches!(&module.functions[0].blocks[0].instructions[0],
            Instruction::ICmp { predicate, ty: Type::Int(16), .. } if *predicate == expected));
    }
}

#[test]
fn pointer_select_phi_and_narrow_signed_gep_index_are_typed() {
    let module = parse_module(
        r#"
@data = global [2 x i8] zeroinitializer
define ptr @choose(i1 %condition, ptr %other) {
entry:
  %last = getelementptr [2 x i8], ptr @data, i32 0, i8 1
  %first = getelementptr i8, ptr %last, i8 -1
  %chosen = select i1 %condition, ptr %first, ptr %other
  br label %end
end:
  %answer = phi ptr [%chosen, %entry]
  ret ptr %answer
}
"#,
    )
    .unwrap();
    assert!(matches!(&module.functions[0].blocks[0].instructions[1],
        Instruction::Gep { indices, .. } if indices == &vec![(Type::Int(8), Operand::Constant(255))]));
    assert!(matches!(
        &module.functions[0].blocks[1].instructions[0],
        Instruction::Phi { ty: Type::Ptr, .. }
    ));
}

#[test]
fn type_layout_is_logical_bytes_not_backend_bit_slots() {
    assert_eq!(Type::Int(1).size_bytes().unwrap(), 1);
    assert_eq!(Type::Int(8).alignment(), 1);
    assert_eq!(Type::Int(16).alignment(), 2);
    assert_eq!(Type::Int(32).alignment(), 4);
    assert_eq!(Type::Ptr.bits(), Some(32));
    let matrix = Type::Array(3, Box::new(Type::Array(2, Box::new(Type::Int(16)))));
    assert_eq!(matrix.size_bytes().unwrap(), 12);
    assert_eq!(matrix.alignment(), 2);
    assert_eq!(matrix.bits(), None);
    assert!(Type::Array(usize::MAX, Box::new(Type::Int(32)))
        .size_bytes()
        .is_err());
}

#[test]
fn malformed_names_attributes_and_layouts_are_rejected() {
    rejected("define i8 @f(i8 %1bad) { ret i8 0 }", 1);
    rejected("define i8 @f(i8 %\"0\") { ret i8 0 }", 1);
    rejected("define void @f() #9 { ret void }", 1);
    rejected("attributes #bad = { nounwind }", 1);
    rejected("attributes #0 = { add i32 1, 2 }", 1);
    rejected("target datalayout = \"e-garbage\"", 1);
    rejected("target datalayout = \"e-p:32:32-p0:32:32\"", 1);
    rejected("target datalayout = \"e-i16:32\"", 1);
    rejected("target datalayout = \"e-ni:0\"", 1);
    rejected("@x = external global i8", 1);
}

#[test]
fn oversized_nesting_is_reported_without_recursing_unboundedly() {
    let source = format!(
        "@x = global {}i8{} zeroinitializer",
        "[1 x ".repeat(130),
        "]".repeat(130)
    );
    rejected(&source, 1);
    let source = format!("!0 = !{}{}", "{".repeat(130), "}".repeat(130));
    rejected(&source, 1);
}

#[test]
fn modern_and_legacy_debug_information_is_explicitly_nonexecuting() {
    let module = parse_module(r#"
declare void @llvm.dbg.declare(metadata, metadata, metadata) #0
declare void @llvm.dbg.value(metadata, metadata, metadata) #0
declare void @llvm.dbg.assign(metadata, metadata, metadata, metadata, metadata, metadata) #0
declare void @llvm.dbg.label(metadata) #0
define i8 @f(i8 %value) !dbg !0 {
entry:
  %p = alloca i8
  call void @llvm.dbg.declare(metadata ptr %p, metadata !1, metadata !DIExpression()), !dbg !2
  call void @llvm.dbg.value(metadata i8 %value, metadata !1, metadata !DIExpression()), !dbg !2
  #dbg_declare(ptr %p, !1, !DIExpression(), !2)
  #dbg_value(i8 %value, !1, !DIExpression(), !2)
  #dbg_assign(i8 %value, !1, !DIExpression(), !3, ptr %p, !DIExpression(), !2)
  #dbg_label(!4, !2)
  call void @llvm.dbg.assign(metadata i8 %value, metadata !1, metadata !DIExpression(), metadata !3, metadata ptr %p, metadata !DIExpression()), !dbg !2
  call void @llvm.dbg.label(metadata !4), !dbg !2
  store i8 %value, ptr %p
  %loaded = load i8, ptr %p
  ret i8 %loaded
}
attributes #0 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
!0 = distinct !DISubprogram(name: "f", line: 1)
!1 = !DILocalVariable(name: "value", scope: !0)
!2 = !DILocation(line: 1, column: 1, scope: !0)
!3 = distinct !DIAssignID()
!4 = !DILabel(name: "entry", scope: !0)
"#).unwrap();
    assert_eq!(module.functions.len(), 1);
    assert_eq!(module.functions[0].blocks[0].instructions.len(), 4);
    assert!(
        matches!(&module.functions[0].blocks[0].instructions[1], Instruction::Store { value: Operand::Local(value), .. } if value == "value")
    );
    rejected(
        "define void @f() {\nentry:\n #dbg_value(i8 1, i8 2, !0, !0)\n ret void\n}",
        3,
    );
    rejected(
        "define void @f() {\nentry:\n #dbg_value(i8 poison, !0, !0, !0)\n ret void\n}",
        3,
    );
    rejected(
        "define void @f() {\nentry:\n #dbg_unknown(i8 1, !0)\n ret void\n}",
        3,
    );
}

#[test]
fn block_comments_and_multiline_literals_preserve_source_lines() {
    let module = parse_module("/* module\ncomment */ @s = global [2 x i8] c\"A\n\"\ndefine i8 @f() {\nentry:\n %x = add/* comment */i8 1, 2\n ret i8 %x\n}").unwrap();
    assert_eq!(module.globals[0].initializer, vec![65, 10]);
    rejected(
        "/* first\n second */\ndefine i8 @f() {\nentry:\n ret i8 %missing\n}",
        5,
    );
    rejected("/* unterminated\ncomment", 1);
}

#[test]
fn zero_literals_and_llvm_signed_hex_constants_keep_their_bits() {
    let module = parse_module(
        r#"
@signed = global i32 s0x80
@unsigned = global i32 u0x80
define i32 @f(ptr %p) {
entry:
  %zero = icmp eq ptr %p, zeroinitializer
  %result = select i1 %zero, i32 -4294967297, i32 zeroinitializer
  ret i32 %result
}
"#,
    )
    .unwrap();
    assert_eq!(module.globals[0].initializer, vec![128, 255, 255, 255]);
    assert_eq!(module.globals[1].initializer, vec![128, 0, 0, 0]);
    assert!(matches!(
        &module.functions[0].blocks[0].instructions[0],
        Instruction::ICmp {
            rhs: Operand::Null,
            ..
        }
    ));
    assert!(matches!(
        &module.functions[0].blocks[0].instructions[1],
        Instruction::Select {
            on_true: Operand::Constant(u32::MAX),
            on_false: Operand::Constant(0),
            ..
        }
    ));
    rejected("define i8 @f() {\nentry:\n ret i8 0xFF\n}", 3);
}

#[test]
fn malformed_modifiers_and_multiline_opcodes_have_source_diagnostics() {
    rejected("define internal external i8 @f() { ret i8 0 }", 1);
    rejected("define common i8 @f() { ret i8 0 }", 1);
    rejected("declare internal i8 @f()", 1);
    rejected(
        "define i8 @f() {\nentry:\n %x =\n freeze i8 1\n ret i8 %x\n}",
        4,
    );
    rejected(
        "define i8 @f() {\nentry:\n %x = call i8 @llvm.dbg.value(i8 1)\n ret i8 %x\n}",
        3,
    );
}

#[test]
fn unresolved_direct_calls_are_typed_for_backend_reachability_checks() {
    let module = parse_module(
        "define i8 @f(i8 %x) {\nentry:\n %y = call i8 @external(i8 %x)\n ret i8 %y\n}",
    )
    .unwrap();
    assert!(matches!(&module.functions[0].blocks[0].instructions[0],
        Instruction::Call { function, return_type: Some(Type::Int(8)), args, .. }
        if function == "external" && args == &vec![(Type::Int(8), Operand::Local("x".into()))]));
}
