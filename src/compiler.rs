//! LLVM integer lowering to the four architectural MOV forms.
//!
//! Scalars are little-endian vectors of RAM words containing 0 or 4. Source
//! pointers are logical byte addresses, never native addresses. Arithmetic is
//! implemented by Boolean MOV networks, not by the host or an MMIO extension.
//!
//! LLVM overflow/exact/inbounds flags constrain defined source executions;
//! lowering chooses wrapping results for integer poison cases. Overshifts
//! deterministically zero-fill (or sign-fill), without eager poison faults.
//! Explicit division by zero, null access, and byte-range violations instead
//! execute an invalid physical load and therefore fault on the actual core.
use crate::llvm::{
    self, BinaryOp, CastOp, Function, Instruction, Module, Operand, Predicate, Type,
};
use crate::{Image, Instruction as Mov, HALT_ADDRESS, PC_ADDRESS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug)]
pub struct CompileOptions {
    pub entry: String,
    pub arguments: Vec<u32>,
    pub ram_words: usize,
    pub memory_bytes: usize,
}
impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            entry: "main".into(),
            arguments: vec![],
            ram_words: 1024,
            memory_bytes: 32,
        }
    }
}
#[derive(Debug)]
pub struct CompiledProgram {
    pub image: Image,
    pub required_ram_words: usize,
    pub logical_memory_bytes: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError(pub String);
impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CompileError {}
type Result<T, E = CompileError> = std::result::Result<T, E>;
fn error(message: impl Into<String>) -> CompileError {
    CompileError(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bit {
    Constant(bool),
    Slot(u32),
}
type Value = Vec<Bit>;
fn constant(value: u32, width: usize) -> Value {
    (0..width)
        .map(|i| Bit::Constant(i < 32 && value & (1u32 << i) != 0))
        .collect()
}
fn literal(value: &[Bit]) -> Option<u32> {
    if value.len() > 32 {
        return None;
    }
    value
        .iter()
        .enumerate()
        .try_fold(0, |number, (i, bit)| match bit {
            Bit::Constant(set) => Some(number | if *set { 1u32 << i } else { 0 }),
            Bit::Slot(_) => None,
        })
}
fn signed_literal(value: u32, bits: usize) -> i64 {
    if value & (1u32 << (bits - 1)) != 0 {
        value as i64 - (1i64 << bits)
    } else {
        value as i64
    }
}
fn fold_binary(op: BinaryOp, a: &[Bit], b: &[Bit]) -> Option<Value> {
    let lhs = literal(a)?;
    let rhs = literal(b)?;
    let n = a.len();
    let sl = signed_literal(lhs, n);
    let sr = signed_literal(rhs, n);
    let result = match op {
        BinaryOp::Add => lhs.wrapping_add(rhs),
        BinaryOp::Sub => lhs.wrapping_sub(rhs),
        BinaryOp::Mul => lhs.wrapping_mul(rhs),
        BinaryOp::And => lhs & rhs,
        BinaryOp::Or => lhs | rhs,
        BinaryOp::Xor => lhs ^ rhs,
        BinaryOp::UDiv | BinaryOp::SDiv | BinaryOp::URem | BinaryOp::SRem if rhs == 0 => {
            return None
        }
        BinaryOp::UDiv => lhs / rhs,
        BinaryOp::URem => lhs % rhs,
        BinaryOp::SDiv => (sl / sr) as u32,
        BinaryOp::SRem => (sl % sr) as u32,
        BinaryOp::Shl => {
            if rhs >= n as u32 {
                0
            } else {
                lhs << rhs
            }
        }
        BinaryOp::LShr => {
            if rhs >= n as u32 {
                0
            } else {
                lhs >> rhs
            }
        }
        BinaryOp::AShr => {
            if rhs >= n as u32 {
                if sl < 0 {
                    u32::MAX
                } else {
                    0
                }
            } else {
                (sl >> rhs) as u32
            }
        }
    };
    Some(constant(result, n))
}
fn width(ty: &Type) -> Result<usize> {
    match ty {
        Type::Int(1 | 8 | 16 | 32) => Ok(ty.bits().unwrap() as usize),
        Type::Ptr => Ok(32),
        _ => Err(error("expected i1/i8/i16/i32 or pointer scalar")),
    }
}
struct Frame<'a> {
    function: &'a Function,
    values: HashMap<String, Value>,
    children: HashMap<(usize, usize), Frame<'a>>,
}
struct Builder<'a> {
    module: &'a Module,
    options: &'a CompileOptions,
    next: usize,
    peak: usize,
    logical_next: usize,
    globals: HashMap<String, Value>,
    initial: Vec<(u32, u32)>,
    code: Vec<Mov>,
    labels: Vec<Option<usize>>,
    relocations: Vec<(usize, usize)>,
    memory: Option<(u32, u32)>, // physical byte-bit buffer, tree root
    needs_memory: bool,
}
impl<'a> Builder<'a> {
    fn allocate(&mut self, count: usize) -> Result<u32> {
        let end = self
            .next
            .checked_add(count)
            .ok_or_else(|| error("RAM layout overflow"))?;
        if end > HALT_ADDRESS as usize / 4 {
            return Err(error("RAM layout overlaps MMIO"));
        }
        let address = (self.next * 4) as u32;
        self.next = end;
        self.peak = self.peak.max(end);
        Ok(address)
    }
    fn vector(&mut self, n: usize) -> Result<Value> {
        let address = self.allocate(n)?;
        Ok((0..n).map(|i| Bit::Slot(address + i as u32 * 4)).collect())
    }
    fn object(&mut self, ty: &Type, count: usize, align: usize) -> Result<u32> {
        if align == 0 || !align.is_power_of_two() {
            return Err(error("invalid object alignment"));
        }
        let size = ty
            .size_bytes()
            .map_err(error)?
            .checked_mul(count)
            .ok_or_else(|| error("source object size overflow"))?;
        let start = self
            .logical_next
            .checked_add(align - 1)
            .ok_or_else(|| error("source address overflow"))?
            & !(align - 1);
        let end = start
            .checked_add(size)
            .ok_or_else(|| error("source address overflow"))?;
        if end > self.options.memory_bytes {
            return Err(error(format!(
                "source objects require {end} logical bytes, capacity is {}",
                self.options.memory_bytes
            )));
        }
        self.logical_next = end;
        self.needs_memory = true;
        Ok(start as u32)
    }
    fn frame(&mut self, name: &str, active: &mut Vec<String>) -> Result<Frame<'a>> {
        if active.iter().any(|n| n == name) {
            return Err(error(format!("recursive call to @{name}")));
        }
        let function = self
            .module
            .functions
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| error(format!("external or unknown function @{name}")))?;
        active.push(name.to_owned());
        let mut values = HashMap::new();
        for (name, ty) in &function.parameters {
            values.insert(name.clone(), self.vector(width(ty)?)?);
        }
        let mut children = HashMap::new();
        for (bi, block) in function.blocks.iter().enumerate() {
            for (ii, instruction) in block.instructions.iter().enumerate() {
                let definition = match instruction {
                    Instruction::Binary { dst, ty, .. }
                    | Instruction::Select { dst, ty, .. }
                    | Instruction::Phi { dst, ty, .. }
                    | Instruction::Load { dst, ty, .. } => Some((dst, width(ty)?)),
                    Instruction::ICmp { dst, .. } => Some((dst, 1)),
                    Instruction::Cast { dst, to, .. } => Some((dst, width(to)?)),
                    Instruction::Gep { dst, .. } => Some((dst, 32)),
                    Instruction::Call {
                        dst: Some(dst),
                        return_type: Some(ty),
                        ..
                    } => Some((dst, width(ty)?)),
                    Instruction::Alloca {
                        dst,
                        ty,
                        count,
                        align,
                    } => {
                        if bi != 0 {
                            return Err(error("static alloca must be in the entry block"));
                        }
                        let address = self.object(ty, *count, (*align).max(ty.alignment()))?;
                        values.insert(dst.clone(), constant(address, 32));
                        None
                    }
                    _ => None,
                };
                if let Some((dst, n)) = definition {
                    values.insert(dst.clone(), self.vector(n)?);
                }
                if matches!(
                    instruction,
                    Instruction::Load { .. } | Instruction::Store { .. }
                ) {
                    self.needs_memory = true;
                }
                if let Instruction::Call {
                    function: callee,
                    args,
                    return_type,
                    ..
                } = instruction
                {
                    let child = self.frame(callee, active)?;
                    if child.function.return_type != *return_type
                        || args.len() != child.function.parameters.len()
                        || args
                            .iter()
                            .zip(&child.function.parameters)
                            .any(|((ty, _), (_, parameter))| ty != parameter)
                    {
                        return Err(error(format!("call signature mismatch for @{callee}")));
                    }
                    children.insert((bi, ii), child);
                }
            }
        }
        active.pop();
        Ok(Frame {
            function,
            values,
            children,
        })
    }
    fn operand<'f>(
        &self,
        frame: &'f Frame<'_>,
        operand: &Operand,
        n: usize,
    ) -> Result<Cow<'f, [Bit]>> {
        match operand {
            Operand::Constant(v) => Ok(Cow::Owned(constant(*v, n))),
            Operand::Null => Ok(Cow::Owned(constant(0, n))),
            Operand::Local(name) => frame
                .values
                .get(name)
                .filter(|v| v.len() == n)
                .map(|v| Cow::Borrowed(v.as_slice()))
                .ok_or_else(|| error(format!("unknown or incorrectly typed local %{name}"))),
            Operand::Global(name) => self
                .globals
                .get(name)
                .filter(|v| v.len() == n)
                .map(|v| Cow::Owned(v.clone()))
                .ok_or_else(|| error(format!("unknown or incorrectly typed global @{name}"))),
        }
    }
    fn emit(&mut self, op: u32, dst: u32, src: u32, extension: u32) {
        self.code.push(Mov::new(op, dst, src, extension));
    }
    fn immediate(&mut self, reg: u32, value: u32) {
        self.emit(1, reg, 0, value);
    }
    fn load(&mut self, reg: u32, bit: Bit) {
        match bit {
            Bit::Constant(v) => self.immediate(reg, if v { 4 } else { 0 }),
            Bit::Slot(address) => self.emit(2, reg, 3, address),
        }
    }
    fn store(&mut self, bit: Bit, reg: u32) {
        if let Bit::Slot(address) = bit {
            self.emit(3, 3, reg, address);
        } else {
            unreachable!("compiler attempted to write a constant");
        }
    }
    fn copy(&mut self, dst: &[Bit], src: &[Bit]) {
        let mut loaded = None;
        for (&d, &s) in dst.iter().zip(src) {
            if d != s {
                if loaded != Some(s) {
                    self.load(0, s);
                    loaded = Some(s);
                }
                self.store(d, 0);
            }
        }
    }
    // Dolan alias comparison: [a]=0; [b]=4; [a] is 4 iff a == b.
    fn eq_bit(&mut self, dst: Bit, a: Bit, b: Bit) {
        if let Bit::Constant(value) = a {
            self.select_bit(dst, b, Bit::Constant(value), Bit::Constant(!value));
            return;
        }
        if let Bit::Constant(value) = b {
            self.select_bit(dst, a, Bit::Constant(value), Bit::Constant(!value));
            return;
        }
        self.load(0, a);
        self.load(1, b);
        self.immediate(2, 0);
        self.emit(3, 0, 2, 0);
        self.immediate(2, 4);
        self.emit(3, 1, 2, 0);
        self.emit(2, 2, 0, 0);
        self.store(dst, 2);
    }
    // A fixed two-element selection table, indexed by an encoded Boolean.
    fn select_bit(&mut self, dst: Bit, condition: Bit, yes: Bit, no: Bit) {
        if yes == no {
            self.copy(&[dst], &[yes]);
            return;
        }
        if let Bit::Constant(value) = condition {
            self.copy(&[dst], &[if value { yes } else { no }]);
            return;
        }
        self.load(0, no);
        self.emit(3, 3, 0, 0);
        self.load(0, yes);
        self.emit(3, 3, 0, 4);
        self.load(1, condition);
        self.emit(2, 0, 1, 0);
        self.store(dst, 0);
    }
    fn gate(&mut self, op: BinaryOp, a: Bit, b: Bit) -> Result<Bit> {
        let known = match (a, b) {
            (Bit::Constant(value), other) | (other, Bit::Constant(value)) => Some((value, other)),
            _ => None,
        };
        if let Some((value, other)) = known {
            return match op {
                BinaryOp::And => Ok(if value { other } else { Bit::Constant(false) }),
                BinaryOp::Or => Ok(if value { Bit::Constant(true) } else { other }),
                BinaryOp::Xor => {
                    if value {
                        self.invert(other)
                    } else {
                        Ok(other)
                    }
                }
                _ => unreachable!(),
            };
        }
        let dst = self.vector(1)?[0];
        match op {
            BinaryOp::And => self.select_bit(dst, a, b, Bit::Constant(false)),
            BinaryOp::Or => self.select_bit(dst, a, Bit::Constant(true), b),
            BinaryOp::Xor => {
                self.eq_bit(dst, a, b);
                self.select_bit(dst, dst, Bit::Constant(false), Bit::Constant(true));
            }
            _ => unreachable!(),
        }
        Ok(dst)
    }
    fn invert(&mut self, a: Bit) -> Result<Bit> {
        if let Bit::Constant(value) = a {
            return Ok(Bit::Constant(!value));
        }
        let dst = self.vector(1)?[0];
        self.select_bit(dst, a, Bit::Constant(false), Bit::Constant(true));
        Ok(dst)
    }
    fn select(&mut self, condition: Bit, yes: &[Bit], no: &[Bit]) -> Result<Value> {
        let out = self.vector(yes.len())?;
        for ((&d, &a), &b) in out.iter().zip(yes).zip(no) {
            self.select_bit(d, condition, a, b);
        }
        Ok(out)
    }
    fn add(&mut self, a: &[Bit], b: &[Bit], subtract: bool) -> Result<Value> {
        if let Some(result) = fold_binary(
            if subtract {
                BinaryOp::Sub
            } else {
                BinaryOp::Add
            },
            a,
            b,
        ) {
            return Ok(result);
        }
        let out = self.vector(a.len())?;
        let mark = self.next;
        let carry = self.vector(1)?[0];
        self.load(0, Bit::Constant(subtract));
        self.store(carry, 0);
        for i in 0..a.len() {
            let iteration = self.next;
            let rhs = if subtract { self.invert(b[i])? } else { b[i] };
            let parity = self.gate(BinaryOp::Xor, a[i], rhs)?;
            let sum = self.gate(BinaryOp::Xor, parity, carry)?;
            let first = self.gate(BinaryOp::And, a[i], rhs)?;
            let second = self.gate(BinaryOp::And, parity, carry)?;
            let next = self.gate(BinaryOp::Or, first, second)?;
            self.copy(&out[i..i + 1], &[sum]);
            self.copy(&[carry], &[next]);
            self.next = iteration;
        }
        self.next = mark;
        Ok(out)
    }
    fn negate(&mut self, value: &[Bit]) -> Result<Value> {
        self.add(&constant(0, value.len()), value, true)
    }
    fn compare(&mut self, a: &[Bit], b: &[Bit], predicate: Predicate) -> Result<Bit> {
        if let (Some(lhs), Some(rhs)) = (literal(a), literal(b)) {
            let sl = signed_literal(lhs, a.len());
            let sr = signed_literal(rhs, b.len());
            return Ok(Bit::Constant(match predicate {
                Predicate::Eq => lhs == rhs,
                Predicate::Ne => lhs != rhs,
                Predicate::Ult => lhs < rhs,
                Predicate::Ule => lhs <= rhs,
                Predicate::Ugt => lhs > rhs,
                Predicate::Uge => lhs >= rhs,
                Predicate::Slt => sl < sr,
                Predicate::Sle => sl <= sr,
                Predicate::Sgt => sl > sr,
                Predicate::Sge => sl >= sr,
            }));
        }
        let dst = self.vector(1)?[0];
        let mark = self.next;
        let eq = self.vector(1)?[0];
        let lt = self.vector(1)?[0];
        self.copy(&[eq], &[Bit::Constant(true)]);
        self.copy(&[lt], &[Bit::Constant(false)]);
        for i in 0..a.len() {
            let iteration = self.next;
            let equal = self.vector(1)?[0];
            self.eq_bit(equal, a[i], b[i]);
            let not_a = self.invert(a[i])?;
            let less = self.gate(BinaryOp::And, not_a, b[i])?;
            self.select_bit(lt, equal, lt, less);
            let all_equal = self.gate(BinaryOp::And, eq, equal)?;
            self.copy(&[eq], &[all_equal]);
            self.next = iteration;
        }
        if matches!(
            predicate,
            Predicate::Slt | Predicate::Sle | Predicate::Sgt | Predicate::Sge
        ) {
            let sign_equal = self.vector(1)?[0];
            self.eq_bit(sign_equal, a[a.len() - 1], b[b.len() - 1]);
            self.select_bit(lt, sign_equal, lt, a[a.len() - 1]);
        }
        let result = match predicate {
            Predicate::Eq => eq,
            Predicate::Ne => self.invert(eq)?,
            Predicate::Ult | Predicate::Slt => lt,
            Predicate::Ule | Predicate::Sle => self.gate(BinaryOp::Or, lt, eq)?,
            Predicate::Ugt | Predicate::Sgt => {
                let le = self.gate(BinaryOp::Or, lt, eq)?;
                self.invert(le)?
            }
            Predicate::Uge | Predicate::Sge => self.invert(lt)?,
        };
        self.copy(&[dst], &[result]);
        self.next = mark;
        Ok(dst)
    }
    fn multiply(&mut self, a: &[Bit], b: &[Bit]) -> Result<Value> {
        if let Some(result) = fold_binary(BinaryOp::Mul, a, b) {
            return Ok(result);
        }
        let out = self.vector(a.len())?;
        let mark = self.next;
        if b.iter().all(|bit| matches!(bit, Bit::Constant(_))) {
            let mut set_bits = b.iter().enumerate().filter_map(|(i, bit)| {
                if matches!(bit, Bit::Constant(true)) {
                    Some(i)
                } else {
                    None
                }
            });
            let first = set_bits.next();
            if set_bits.next().is_none() {
                for i in 0..out.len() {
                    let source = first
                        .and_then(|shift| i.checked_sub(shift))
                        .map(|j| a[j])
                        .unwrap_or(Bit::Constant(false));
                    self.copy(&out[i..i + 1], &[source]);
                }
                return Ok(out);
            }
        }
        self.copy(&out, &constant(0, a.len()));
        let row = self.vector(a.len())?;
        for j in 0..b.len() {
            if matches!(b[j], Bit::Constant(false)) {
                continue;
            }
            let iteration = self.next;
            for i in 0..a.len() {
                self.select_bit(
                    row[i],
                    b[j],
                    if i >= j {
                        a[i - j]
                    } else {
                        Bit::Constant(false)
                    },
                    Bit::Constant(false),
                );
            }
            let sum = self.add(&out, &row, false)?;
            self.copy(&out, &sum);
            self.next = iteration;
        }
        self.next = mark;
        Ok(out)
    }
    fn shift(&mut self, a: &[Bit], b: &[Bit], op: BinaryOp) -> Result<Value> {
        let out = self.vector(a.len())?;
        let mark = self.next;
        self.copy(&out, a);
        let low_stages = a.len().trailing_zeros() as usize;
        for (k, &selector) in b.iter().take(low_stages).enumerate() {
            let distance = 1usize << k;
            // Read unmodified source bits before overwriting their slots.
            for step in 0..a.len() {
                let i = if matches!(op, BinaryOp::Shl) {
                    a.len() - 1 - step
                } else {
                    step
                };
                let source = match op {
                    BinaryOp::Shl => i.checked_sub(distance).map(|j| out[j]),
                    _ => i
                        .checked_add(distance)
                        .filter(|&j| j < a.len())
                        .map(|j| out[j]),
                }
                .unwrap_or(if matches!(op, BinaryOp::AShr) {
                    out[a.len() - 1]
                } else {
                    Bit::Constant(false)
                });
                self.select_bit(out[i], selector, source, out[i]);
            }
        }
        if b.len() > low_stages {
            let high = self.vector(1)?[0];
            self.copy(&[high], &[Bit::Constant(false)]);
            for &selector in b.iter().skip(low_stages) {
                let iteration = self.next;
                let combined = self.gate(BinaryOp::Or, high, selector)?;
                self.copy(&[high], &[combined]);
                self.next = iteration;
            }
            let fill = if matches!(op, BinaryOp::AShr) {
                a[a.len() - 1]
            } else {
                Bit::Constant(false)
            };
            for &bit in &out {
                self.select_bit(bit, high, fill, bit);
            }
        }
        self.next = mark;
        Ok(out)
    }
    fn label(&mut self) -> usize {
        let label = self.labels.len();
        self.labels.push(None);
        label
    }
    fn bind(&mut self, label: usize) {
        self.labels[label] = Some(self.code.len());
    }
    fn label_load(&mut self, reg: u32, label: usize) {
        self.relocations.push((self.code.len(), label));
        self.immediate(reg, 0);
    }
    fn jump(&mut self, label: usize) {
        self.label_load(0, label);
        self.emit(3, 3, 0, PC_ADDRESS);
    }
    fn branch(&mut self, condition: Bit, yes: usize, no: usize) {
        if let Bit::Constant(value) = condition {
            self.jump(if value { yes } else { no });
            return;
        }
        self.label_load(0, no);
        self.emit(3, 3, 0, 0);
        self.label_load(0, yes);
        self.emit(3, 3, 0, 4);
        self.load(1, condition);
        self.emit(2, 0, 1, 0);
        self.emit(3, 3, 0, PC_ADDRESS);
    }
    fn fault_if(&mut self, condition: Bit) {
        if let Bit::Constant(value) = condition {
            if value {
                self.immediate(0, 0xffff_fff4);
                self.emit(2, 0, 0, 0);
            }
            return;
        }
        let fault = self.label();
        let continuation = self.label();
        self.branch(condition, fault, continuation);
        self.bind(fault);
        self.immediate(0, 0xffff_fff4);
        self.emit(2, 0, 0, 0);
        self.bind(continuation);
    }
    fn divide(&mut self, a: &[Bit], b: &[Bit], signed: bool, remainder: bool) -> Result<Value> {
        let out = self.vector(a.len())?;
        let mark = self.next;
        let zero = self.compare(b, &constant(0, b.len()), Predicate::Eq)?;
        self.fault_if(zero);
        let (numerator, denominator) = if signed {
            let neg_a = self.negate(a)?;
            let neg_b = self.negate(b)?;
            (
                self.select(a[a.len() - 1], &neg_a, a)?,
                self.select(b[b.len() - 1], &neg_b, b)?,
            )
        } else {
            (a.to_vec(), b.to_vec())
        };
        let rem = self.vector(a.len() + 1)?;
        self.copy(&rem, &constant(0, rem.len()));
        let quotient = self.vector(a.len())?;
        let mut divisor = denominator;
        divisor.push(Bit::Constant(false));
        for i in (0..a.len()).rev() {
            let iteration = self.next;
            for j in (1..rem.len()).rev() {
                self.copy(&rem[j..j + 1], &rem[j - 1..j]);
            }
            self.copy(&rem[..1], &numerator[i..i + 1]);
            let ge = self.compare(&rem, &divisor, Predicate::Uge)?;
            let difference = self.add(&rem, &divisor, true)?;
            let selected = self.select(ge, &difference, &rem)?;
            self.copy(&rem, &selected);
            self.copy(&quotient[i..i + 1], &[ge]);
            self.next = iteration;
        }
        let magnitude = if remainder {
            &rem[..a.len()]
        } else {
            &quotient[..]
        };
        if signed {
            let sign = if remainder {
                a[a.len() - 1]
            } else {
                self.gate(BinaryOp::Xor, a[a.len() - 1], b[b.len() - 1])?
            };
            let negative = self.negate(magnitude)?;
            let selected = self.select(sign, &negative, magnitude)?;
            self.copy(&out, &selected);
        } else {
            self.copy(&out, magnitude);
        }
        self.next = mark;
        Ok(out)
    }
    fn binary(&mut self, op: BinaryOp, a: &[Bit], b: &[Bit]) -> Result<Value> {
        if let Some(result) = fold_binary(op, a, b) {
            return Ok(result);
        }
        match op {
            BinaryOp::Add => self.add(a, b, false),
            BinaryOp::Sub => self.add(a, b, true),
            BinaryOp::Mul => self.multiply(a, b),
            BinaryOp::UDiv => self.divide(a, b, false, false),
            BinaryOp::URem => self.divide(a, b, false, true),
            BinaryOp::SDiv => self.divide(a, b, true, false),
            BinaryOp::SRem => self.divide(a, b, true, true),
            BinaryOp::Shl | BinaryOp::LShr | BinaryOp::AShr => self.shift(a, b, op),
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                let out = self.vector(a.len())?;
                let mark = self.next;
                for i in 0..a.len() {
                    let v = self.gate(op, a[i], b[i])?;
                    self.copy(&out[i..i + 1], &[v]);
                    self.next = mark;
                }
                Ok(out)
            }
        }
    }
    fn initialize_memory(&mut self) -> Result<()> {
        if !self.needs_memory {
            return Ok(());
        }
        let buffer = self.allocate(self.options.memory_bytes * 8)?;
        let root = self.tree(0, self.options.memory_bytes, buffer)?;
        self.memory = Some((buffer, root));
        for global in &self.module.globals {
            let address =
                self.globals[&global.name]
                    .iter()
                    .enumerate()
                    .fold(0usize, |v, (i, b)| {
                        v | if matches!(b, Bit::Constant(true)) {
                            1 << i
                        } else {
                            0
                        }
                    });
            if global.initializer.len() != global.ty.size_bytes().map_err(error)? {
                return Err(error("global initializer size mismatch"));
            }
            for (i, &byte) in global.initializer.iter().enumerate() {
                for bit in 0..8 {
                    if byte & (1 << bit) != 0 {
                        self.initial
                            .push((buffer + ((address + i) * 8 + bit) as u32 * 4, 4));
                    }
                }
            }
        }
        Ok(())
    }
    fn tree(&mut self, first: usize, count: usize, buffer: u32) -> Result<u32> {
        if count == 1 {
            return Ok(buffer + first as u32 * 32);
        }
        let node = self.allocate(2)?;
        let low = self.tree(first, count / 2, buffer)?;
        let high = self.tree(first + count / 2, count / 2, buffer)?;
        self.initial.push((node, low));
        self.initial.push((node + 4, high));
        Ok(node)
    }
    // Bounds are checked in logical bytes before turning a bitvector into a
    // native leaf pointer. The tree consumes only the low address bits.
    fn memory_pointer(&mut self, pointer: &[Bit], bytes: usize) -> Result<Bit> {
        let (_, root) = self.memory.ok_or_else(|| error("memory layout missing"))?;
        let null = self.compare(pointer, &constant(0, 32), Predicate::Eq)?;
        self.fault_if(null);
        let max = self
            .options
            .memory_bytes
            .checked_sub(bytes)
            .ok_or_else(|| error("memory access wider than source memory"))?;
        let invalid = self.compare(pointer, &constant(max as u32, 32), Predicate::Ugt)?;
        self.fault_if(invalid);
        let native = self.vector(1)?[0];
        self.immediate(0, root);
        self.store(native, 0);
        for i in (0..self.options.memory_bytes.trailing_zeros() as usize).rev() {
            self.load(2, native);
            self.emit(2, 0, 2, 0);
            self.emit(3, 3, 0, 0);
            self.emit(2, 0, 2, 4);
            self.emit(3, 3, 0, 4);
            self.load(1, pointer[i]);
            self.emit(2, 0, 1, 0);
            self.store(native, 0);
        }
        Ok(native)
    }
    fn edge(
        &mut self,
        frame: &Frame<'_>,
        from: &str,
        to: &str,
        labels: &HashMap<String, usize>,
    ) -> Result<()> {
        let block = frame
            .function
            .blocks
            .iter()
            .find(|b| b.label == to)
            .ok_or_else(|| error(format!("unknown block %{to}")))?;
        let mut moves = Vec::new();
        for instruction in &block.instructions {
            if let Instruction::Phi { dst, ty, incoming } = instruction {
                let value = incoming
                    .iter()
                    .find(|(_, label)| label == from)
                    .ok_or_else(|| error("missing PHI predecessor"))?;
                let src = self.operand(frame, &value.0, width(ty)?)?;
                let temporary = self.vector(src.len())?;
                self.copy(&temporary, &src);
                moves.push((frame.values[dst].as_slice(), temporary));
            }
        }
        for (dst, src) in moves {
            self.copy(dst, &src);
        }
        self.jump(
            *labels
                .get(to)
                .ok_or_else(|| error("unknown branch label"))?,
        );
        Ok(())
    }
    fn emit_frame(
        &mut self,
        frame: &Frame<'_>,
        return_value: &[Bit],
        continuation: Option<usize>,
        result_address: u32,
    ) -> Result<()> {
        let labels: HashMap<String, usize> = frame
            .function
            .blocks
            .iter()
            .map(|b| (b.label.clone(), self.label()))
            .collect();
        if frame.function.blocks.is_empty() {
            return Err(error("function has no blocks"));
        }
        for (bi, block) in frame.function.blocks.iter().enumerate() {
            self.bind(labels[&block.label]);
            for (ii, instruction) in block.instructions.iter().enumerate() {
                let mark = self.next;
                match instruction {
                    Instruction::Phi { .. } | Instruction::Alloca { .. } => {}
                    Instruction::Binary {
                        dst,
                        op,
                        ty,
                        lhs,
                        rhs,
                    } => {
                        let n = width(ty)?;
                        let a = self.operand(frame, lhs, n)?;
                        let b = self.operand(frame, rhs, n)?;
                        let out = self.binary(*op, &a, &b)?;
                        self.copy(&frame.values[dst], &out);
                    }
                    Instruction::ICmp {
                        dst,
                        predicate,
                        ty,
                        lhs,
                        rhs,
                    } => {
                        let n = width(ty)?;
                        let a = self.operand(frame, lhs, n)?;
                        let b = self.operand(frame, rhs, n)?;
                        let out = self.compare(&a, &b, *predicate)?;
                        self.copy(&frame.values[dst], &[out]);
                    }
                    Instruction::Select {
                        dst,
                        condition,
                        ty,
                        on_true,
                        on_false,
                    } => {
                        let n = width(ty)?;
                        let c = self.operand(frame, condition, 1)?[0];
                        let a = self.operand(frame, on_true, n)?;
                        let b = self.operand(frame, on_false, n)?;
                        let out = self.select(c, &a, &b)?;
                        self.copy(&frame.values[dst], &out);
                    }
                    Instruction::Cast {
                        dst,
                        op,
                        from,
                        value,
                        to,
                    } => {
                        let a = self.operand(frame, value, width(from)?)?;
                        let n = width(to)?;
                        let extension = if matches!(op, CastOp::SExt) {
                            a[a.len() - 1]
                        } else {
                            Bit::Constant(false)
                        };
                        let out: Value = (0..n)
                            .map(|i| a.get(i).copied().unwrap_or(extension))
                            .collect();
                        self.copy(&frame.values[dst], &out);
                    }
                    Instruction::Load {
                        dst, ty, pointer, ..
                    } => {
                        let n = width(ty)?;
                        let pointer = self.operand(frame, pointer, 32)?;
                        let native =
                            self.memory_pointer(&pointer, ty.size_bytes().map_err(error)?)?;
                        self.load(1, native);
                        for (i, &bit) in frame.values[dst].iter().enumerate().take(n) {
                            self.emit(2, 0, 1, i as u32 * 4);
                            self.store(bit, 0);
                        }
                    }
                    Instruction::Store {
                        ty, value, pointer, ..
                    } => {
                        let n = width(ty)?;
                        let value = self.operand(frame, value, n)?;
                        let pointer = self.operand(frame, pointer, 32)?;
                        let native =
                            self.memory_pointer(&pointer, ty.size_bytes().map_err(error)?)?;
                        self.load(1, native);
                        for (i, &bit) in value.iter().enumerate() {
                            self.load(0, bit);
                            self.emit(3, 1, 0, i as u32 * 4);
                        }
                    }
                    Instruction::Gep {
                        dst,
                        source_type,
                        base,
                        indices,
                    } => {
                        let mut pointer = self.operand(frame, base, 32)?;
                        let mut current = source_type.clone();
                        for (index, (ty, operand)) in indices.iter().enumerate() {
                            let value = self.operand(frame, operand, width(ty)?)?;
                            let signed: Value = (0..32)
                                .map(|i| value.get(i).copied().unwrap_or(value[value.len() - 1]))
                                .collect();
                            if index > 0 {
                                current = match current {
                                    Type::Array(_, element) => *element,
                                    _ => return Err(error("GEP indexes a non-array element")),
                                };
                            }
                            let size = current.size_bytes().map_err(error)?;
                            if size > u32::MAX as usize {
                                return Err(error("GEP element is too large"));
                            }
                            let offset = self.multiply(&signed, &constant(size as u32, 32))?;
                            pointer = Cow::Owned(self.add(&pointer, &offset, false)?);
                        }
                        self.copy(&frame.values[dst], &pointer);
                    }
                    Instruction::Call { dst, args, .. } => {
                        let child = &frame.children[&(bi, ii)];
                        for ((ty, operand), (name, _)) in
                            args.iter().zip(&child.function.parameters)
                        {
                            let src = self.operand(frame, operand, width(ty)?)?;
                            self.copy(&child.values[name], &src);
                        }
                        let resume = self.label();
                        let target = dst
                            .as_ref()
                            .map(|name| frame.values[name].as_slice())
                            .unwrap_or(&[]);
                        self.emit_frame(child, target, Some(resume), result_address)?;
                        self.bind(resume);
                    }
                    Instruction::Br {
                        condition,
                        on_true,
                        on_false,
                    } => {
                        if let Some(condition) = condition {
                            let c = self.operand(frame, condition, 1)?[0];
                            let yes = self.label();
                            let no = self.label();
                            self.branch(c, yes, no);
                            self.bind(yes);
                            self.edge(frame, &block.label, on_true, &labels)?;
                            self.next = mark;
                            self.bind(no);
                            self.edge(
                                frame,
                                &block.label,
                                on_false.as_ref().ok_or_else(|| {
                                    error("conditional branch missing false target")
                                })?,
                                &labels,
                            )?;
                        } else {
                            self.edge(frame, &block.label, on_true, &labels)?;
                        }
                    }
                    Instruction::Ret { ty, value } => {
                        if let (Some(ty), Some(value)) = (ty, value) {
                            let src = self.operand(frame, value, width(ty)?)?;
                            if !return_value.is_empty() {
                                self.copy(return_value, &src);
                            }
                        } else if continuation.is_none() {
                            self.copy(return_value, &constant(0, return_value.len()));
                        }
                        if let Some(label) = continuation {
                            self.jump(label);
                        } else {
                            self.immediate(0, result_address);
                            self.emit(3, 3, 0, HALT_ADDRESS);
                        }
                    }
                    Instruction::Unreachable => {
                        self.immediate(0, 0xffff_fff4);
                        self.emit(2, 0, 0, 0);
                    }
                }
                self.next = mark;
            }
        }
        Ok(())
    }
}

pub fn compile_ir(source: &str, options: &CompileOptions) -> Result<CompiledProgram> {
    let module = llvm::parse_module(source).map_err(|e| error(e.to_string()))?;
    compile_module(&module, options)
}
fn compile_module(module: &Module, options: &CompileOptions) -> Result<CompiledProgram> {
    if !(2..=128).contains(&options.memory_bytes) || !options.memory_bytes.is_power_of_two() {
        return Err(error(
            "memory_bytes must be a power of two from 2 through 128",
        ));
    }
    if options.ram_words > HALT_ADDRESS as usize / 4 {
        return Err(error("RAM capacity overlaps MMIO"));
    }
    let mut builder = Builder {
        module,
        options,
        next: 2,
        peak: 2,
        logical_next: 1,
        globals: HashMap::new(),
        initial: vec![],
        code: vec![],
        labels: vec![],
        relocations: vec![],
        memory: None,
        needs_memory: false,
    };
    for global in &module.globals {
        let address = builder.object(&global.ty, 1, global.align.max(global.ty.alignment()))?;
        builder
            .globals
            .insert(global.name.clone(), constant(address, 32));
    }
    let frame = builder.frame(&options.entry, &mut vec![])?;
    if frame.function.parameters.len() != options.arguments.len() {
        return Err(error(format!(
            "entry @{} expects {} arguments, got {}",
            options.entry,
            frame.function.parameters.len(),
            options.arguments.len()
        )));
    }
    let result_bits = frame
        .function
        .return_type
        .as_ref()
        .map(width)
        .transpose()?
        .unwrap_or(1);
    let result = builder.vector(result_bits)?;
    let result_address = match result[0] {
        Bit::Slot(address) => address,
        _ => unreachable!(),
    };
    builder.initialize_memory()?;
    for ((name, ty), &argument) in frame.function.parameters.iter().zip(&options.arguments) {
        for (&slot, bit) in frame.values[name]
            .iter()
            .zip(constant(argument, width(ty)?))
        {
            if let (Bit::Slot(address), Bit::Constant(true)) = (slot, bit) {
                builder.initial.push((address, 4));
            }
        }
    }
    builder.emit_frame(&frame, &result, None, result_address)?;
    let required_ram_words = builder.peak;
    if required_ram_words > options.ram_words {
        return Err(error(format!(
            "compiled program requires {required_ram_words} RAM words, capacity is {}",
            options.ram_words
        )));
    }
    // The CPU does not clear RAM on reset. Initialize all private storage on
    // every entry, including global bytes, frame values, and reusable scratch.
    let mut startup = vec![Mov::new(1, 3, 0, 0), Mov::new(1, 0, 0, 0)];
    for address in 0..required_ram_words {
        startup.push(Mov::new(3, 3, 0, (address * 4) as u32));
    }
    let mut initial_value = 0;
    for (address, value) in builder.initial {
        if value != initial_value {
            startup.push(Mov::new(1, 0, 0, value));
            initial_value = value;
        }
        startup.push(Mov::new(3, 3, 0, address));
    }
    let prefix = startup.len();
    if prefix
        .checked_add(builder.code.len())
        .filter(|&n| n <= u32::MAX as usize)
        .is_none()
    {
        return Err(error("compiled code exceeds instruction address space"));
    }
    for (instruction, label) in builder.relocations {
        let target = builder.labels[label].ok_or_else(|| error("unbound compiler label"))?;
        builder.code[instruction].extension = (target + prefix) as u32;
    }
    startup.extend(builder.code);
    let image = Image::compiled(
        startup,
        required_ram_words,
        result_address,
        result_bits as u8,
    )
    .map_err(|e| error(e.to_string()))?;
    Ok(CompiledProgram {
        image,
        required_ram_words,
        logical_memory_bytes: if builder.needs_memory {
            options.memory_bytes
        } else {
            0
        },
    })
}
