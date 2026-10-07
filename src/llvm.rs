//! A dependency-free parser for the executable LLVM subset used by the MOV backend.
//!
//! Source pointers are opaque, little-endian, 32-bit logical byte addresses. LLVM
//! attributes and metadata are retained only insofar as they constrain parsing;
//! unsupported executable syntax is always an error, never an ignored instruction.
//!
//! Supported values are i1/i8/i16/i32 and opaque pointers; arrays are storage
//! types. The grammar includes integer arithmetic/comparisons/casts/select,
//! edge-checked PHIs, branches/returns/unreachable, direct nonvariadic calls,
//! entry-block static allocas, scalar loads/stores, literal globals and array
//! GEPs. Instruction flags express source definedness, not extra operations.
//! Debug metadata, modern debug records, legacy debug intrinsics, source
//! bookkeeping and common Clang attributes are accepted as nonexecuting data.
//! Atomics/volatile accesses, exceptions, vectors/structs/floats, indirect calls,
//! dynamic allocation, nonzero address spaces, ABI-copy attributes, relocatable
//! global initializers and executable constant expressions are unsupported.
//! Quoted identifiers are supported except all-numeric names, which the public
//! string-based AST cannot distinguish from numbered local/global identifiers.

use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}
impl std::error::Error for ParseError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Int(u8),
    Ptr,
    Array(usize, Box<Type>),
}
impl Type {
    pub fn bits(&self) -> Option<u8> {
        match self {
            Self::Int(width) => Some(*width),
            Self::Ptr => Some(32),
            Self::Array(..) => None,
        }
    }
    pub fn size_bytes(&self) -> Result<usize, String> {
        match self {
            Self::Int(1 | 8) => Ok(1),
            Self::Int(16) => Ok(2),
            Self::Int(32) | Self::Ptr => Ok(4),
            Self::Int(width) => Err(format!("unsupported integer width i{width}")),
            Self::Array(count, element) => count
                .checked_mul(element.size_bytes()?)
                .ok_or_else(|| "type size overflows the host address space".into()),
        }
    }
    pub fn alignment(&self) -> usize {
        match self {
            Self::Int(width) => usize::from(*width).div_ceil(8).clamp(1, 4),
            Self::Ptr => 4,
            Self::Array(_, element) => element.alignment(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    Local(String),
    Global(String),
    Constant(u32),
    Null,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    UDiv,
    SDiv,
    URem,
    SRem,
    And,
    Or,
    Xor,
    Shl,
    LShr,
    AShr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Predicate {
    Eq,
    Ne,
    Ult,
    Ule,
    Ugt,
    Uge,
    Slt,
    Sle,
    Sgt,
    Sge,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastOp {
    Trunc,
    ZExt,
    SExt,
    Bitcast,
    PtrToInt,
    IntToPtr,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instruction {
    Binary {
        dst: String,
        op: BinaryOp,
        ty: Type,
        lhs: Operand,
        rhs: Operand,
    },
    ICmp {
        dst: String,
        predicate: Predicate,
        ty: Type,
        lhs: Operand,
        rhs: Operand,
    },
    Select {
        dst: String,
        condition: Operand,
        ty: Type,
        on_true: Operand,
        on_false: Operand,
    },
    Cast {
        dst: String,
        op: CastOp,
        from: Type,
        value: Operand,
        to: Type,
    },
    Phi {
        dst: String,
        ty: Type,
        incoming: Vec<(Operand, String)>,
    },
    Alloca {
        dst: String,
        ty: Type,
        count: usize,
        align: usize,
    },
    Load {
        dst: String,
        ty: Type,
        pointer: Operand,
        align: usize,
    },
    Store {
        ty: Type,
        value: Operand,
        pointer: Operand,
        align: usize,
    },
    Gep {
        dst: String,
        source_type: Type,
        base: Operand,
        indices: Vec<(Type, Operand)>,
    },
    Call {
        dst: Option<String>,
        function: String,
        args: Vec<(Type, Operand)>,
        return_type: Option<Type>,
    },
    Br {
        condition: Option<Operand>,
        on_true: String,
        on_false: Option<String>,
    },
    Ret {
        ty: Option<Type>,
        value: Option<Operand>,
    },
    Unreachable,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub label: String,
    pub instructions: Vec<Instruction>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub return_type: Option<Type>,
    pub parameters: Vec<(String, Type)>,
    pub blocks: Vec<Block>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Global {
    pub name: String,
    pub ty: Type,
    pub initializer: Vec<u8>,
    pub align: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub functions: Vec<Function>,
    pub globals: Vec<Global>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Word(String),
    Local(String),
    Global(String),
    String(Vec<u8>),
    Metadata(String),
    Group(String),
    Debug(String),
    Symbol(char),
}
#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    line: usize,
}
fn error(line: usize, message: impl Into<String>) -> ParseError {
    ParseError {
        line,
        message: message.into(),
    }
}
fn debug_arity(name: &str) -> Option<usize> {
    match name {
        "llvm.dbg.value" | "llvm.dbg.declare" | "llvm.dbg.addr" => Some(3),
        "llvm.dbg.assign" => Some(6),
        "llvm.dbg.label" => Some(1),
        _ => None,
    }
}
fn debug_record_arity(name: &str) -> Option<usize> {
    match name {
        "dbg_value" | "dbg_declare" => Some(4),
        "dbg_assign" => Some(7),
        "dbg_label" => Some(2),
        _ => None,
    }
}
fn name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'$' | b'.' | b'_')
}
fn bare_name(name: &str, line: usize) -> Result<String, ParseError> {
    if name.is_empty() || !name.bytes().all(name_byte) {
        return Err(error(line, "invalid identifier"));
    }
    if name.as_bytes()[0].is_ascii_digit() {
        if !name.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(error(
                line,
                "numbered identifiers contain only decimal digits",
            ));
        }
        return name
            .parse::<usize>()
            .map(|number| number.to_string())
            .map_err(|_| error(line, "numbered identifier is too large"));
    }
    Ok(name.to_owned())
}
fn block_name(name: &str, line: usize) -> Result<String, ParseError> {
    if !name.is_empty()
        && name.bytes().all(name_byte)
        && !name.bytes().all(|byte| byte.is_ascii_digit())
    {
        Ok(name.to_owned())
    } else {
        bare_name(name, line)
    }
}
fn quoted_name(bytes: Vec<u8>, line: usize) -> Result<String, ParseError> {
    let name = String::from_utf8(bytes).map_err(|_| error(line, "identifier is not UTF-8"))?;
    if name.is_empty() || name.contains('\0') {
        return Err(error(line, "empty or NUL-containing identifier"));
    }
    if name.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(
            line,
            "quoted all-numeric identifiers are unsupported",
        ));
    }
    Ok(name)
}
fn lex(source: &str) -> Result<Vec<Token>, ParseError> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut pos = 0;
    let mut line = 1;
    while pos < bytes.len() {
        let start_line = line;
        let byte = bytes[pos];
        if byte.is_ascii_whitespace() {
            line += usize::from(byte == b'\n');
            pos += 1;
            continue;
        }
        if byte == b';' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(pos + 1) == Some(&b'*') {
            pos += 2;
            let mut closed = false;
            while pos < bytes.len() {
                if bytes[pos] == b'*' && bytes.get(pos + 1) == Some(&b'/') {
                    pos += 2;
                    closed = true;
                    break;
                }
                line += usize::from(bytes[pos] == b'\n');
                pos += 1;
            }
            if !closed {
                return Err(error(start_line, "unterminated block comment"));
            }
            continue;
        }
        let prefix = matches!(byte, b'%' | b'@' | b'!' | b'#').then_some(byte);
        if prefix.is_some() {
            pos += 1;
        }
        let kind = if bytes.get(pos) == Some(&b'"') {
            pos += 1;
            let mut value = Vec::new();
            let mut closed = false;
            while pos < bytes.len() {
                match bytes[pos] {
                    b'"' => {
                        pos += 1;
                        closed = true;
                        break;
                    }
                    b'\n' => {
                        value.push(b'\n');
                        line += 1;
                        pos += 1;
                    }
                    b'\\' => {
                        pos += 1;
                        let Some(&first) = bytes.get(pos) else {
                            break;
                        };
                        if let (Some(high), Some(low)) = (
                            (first as char).to_digit(16),
                            bytes.get(pos + 1).and_then(|b| (*b as char).to_digit(16)),
                        ) {
                            value.push((high * 16 + low) as u8);
                            pos += 2;
                        } else if first == b'\\' {
                            value.push(first);
                            pos += 1;
                        } else {
                            return Err(error(
                                start_line,
                                "invalid string escape (expected two hexadecimal digits)",
                            ));
                        }
                    }
                    other => {
                        value.push(other);
                        pos += 1;
                    }
                }
            }
            if !closed {
                return Err(error(start_line, "unterminated quoted string"));
            }
            match prefix {
                Some(b'%' | b'@') => {
                    let name = quoted_name(value, start_line)?;
                    if prefix == Some(b'%') {
                        Kind::Local(name)
                    } else {
                        Kind::Global(name)
                    }
                }
                Some(b'!') => {
                    tokens.push(Token {
                        kind: Kind::Symbol('!'),
                        line: start_line,
                    });
                    Kind::String(value)
                }
                Some(_) => return Err(error(start_line, "invalid quoted attribute identifier")),
                None => Kind::String(value),
            }
        } else if let Some(prefix) = prefix {
            let start = pos;
            while pos < bytes.len() && name_byte(bytes[pos]) {
                pos += 1;
            }
            let name = std::str::from_utf8(&bytes[start..pos]).unwrap().to_owned();
            if name.is_empty() {
                if prefix == b'!' {
                    Kind::Symbol('!')
                } else {
                    return Err(error(start_line, "missing identifier after sigil"));
                }
            } else {
                let name = bare_name(&name, start_line)?;
                if prefix == b'#'
                    && !name.bytes().all(|byte| byte.is_ascii_digit())
                    && debug_record_arity(&name).is_none()
                {
                    return Err(error(
                        start_line,
                        "attribute group identifier must be numeric",
                    ));
                }
                match prefix {
                    b'%' => Kind::Local(name),
                    b'@' => Kind::Global(name),
                    b'!' => Kind::Metadata(name),
                    b'#' if debug_record_arity(&name).is_some() => Kind::Debug(name),
                    b'#' => Kind::Group(name),
                    _ => unreachable!(),
                }
            }
        } else if b"=,[]{}():*<>".contains(&byte) {
            pos += 1;
            Kind::Symbol(byte as char)
        } else {
            let start = pos;
            while pos < bytes.len()
                && !bytes[pos].is_ascii_whitespace()
                && !b";/=,[]{}():*<>%@!#\"".contains(&bytes[pos])
            {
                pos += 1;
            }
            if pos == start {
                return Err(error(start_line, "unexpected character"));
            }
            let word = std::str::from_utf8(&bytes[start..pos])
                .map_err(|_| error(start_line, "invalid token"))?;
            Kind::Word(word.to_owned())
        };
        tokens.push(Token {
            kind,
            line: start_line,
        });
    }
    Ok(tokens)
}

#[derive(Clone)]
struct Signature {
    return_type: Option<Type>,
    parameters: Vec<Type>,
    variadic: bool,
    defined: bool,
}
struct FunctionLines {
    line: usize,
    blocks: Vec<usize>,
    instructions: Vec<Vec<usize>>,
}
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    eof_line: usize,
    signatures: HashMap<String, Signature>,
    global_names: HashSet<String>,
    debug_declarations: HashSet<String>,
    lines: Vec<FunctionLines>,
    next_slot: usize,
}
impl Parser {
    fn line(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map_or(self.eof_line, |token| token.line)
    }
    fn fail<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(error(self.line(), message))
    }
    fn peek(&self) -> Option<&Kind> {
        self.tokens.get(self.pos).map(|token| &token.kind)
    }
    fn word_is(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Kind::Word(value)) if value == word)
    }
    fn eat_word(&mut self, word: &str) -> bool {
        if self.word_is(word) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_word(&mut self, word: &str) -> Result<(), ParseError> {
        if self.eat_word(word) {
            Ok(())
        } else {
            self.fail(format!("expected '{word}'"))
        }
    }
    fn symbol_is(&self, symbol: char) -> bool {
        self.peek() == Some(&Kind::Symbol(symbol))
    }
    fn eat_symbol(&mut self, symbol: char) -> bool {
        if self.symbol_is(symbol) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_symbol(&mut self, symbol: char) -> Result<(), ParseError> {
        if self.eat_symbol(symbol) {
            Ok(())
        } else {
            self.fail(format!("expected '{symbol}'"))
        }
    }
    fn word(&mut self) -> Result<String, ParseError> {
        if let Some(Kind::Word(value)) = self.peek() {
            let value = value.clone();
            self.pos += 1;
            Ok(value)
        } else {
            self.fail("expected a keyword or integer")
        }
    }
    fn local(&mut self) -> Result<String, ParseError> {
        if let Some(Kind::Local(value)) = self.peek() {
            let value = value.clone();
            self.pos += 1;
            Ok(value)
        } else {
            self.fail("expected a local identifier")
        }
    }
    fn global(&mut self) -> Result<String, ParseError> {
        if let Some(Kind::Global(value)) = self.peek() {
            let value = value.clone();
            self.pos += 1;
            Ok(value)
        } else {
            self.fail("expected a global identifier")
        }
    }
    fn string(&mut self) -> Result<Vec<u8>, ParseError> {
        if let Some(Kind::String(value)) = self.peek() {
            let value = value.clone();
            self.pos += 1;
            Ok(value)
        } else {
            self.fail("expected a quoted string")
        }
    }
    fn natural(&mut self) -> Result<usize, ParseError> {
        let line = self.line();
        let word = self.word()?;
        if !word.bytes().all(|b| b.is_ascii_digit()) {
            return Err(error(line, "expected a nonnegative decimal integer"));
        }
        word.parse()
            .map_err(|_| error(line, "integer exceeds the host address space"))
    }
    fn alignment(&mut self) -> Result<usize, ParseError> {
        let line = self.line();
        let value = self.natural()?;
        if value == 0 || !value.is_power_of_two() || value > (1usize << 32) {
            return Err(error(
                line,
                "alignment must be a power of two no greater than 2^32",
            ));
        }
        Ok(value)
    }
    fn ty(&mut self) -> Result<Type, ParseError> {
        self.ty_nested(0)
    }
    fn ty_nested(&mut self, depth: usize) -> Result<Type, ParseError> {
        if depth > 128 {
            return self.fail("type nesting exceeds 128 levels");
        }
        let line = self.line();
        if self.eat_symbol('[') {
            let count = self.natural()?;
            self.expect_word("x")?;
            let element = self.ty_nested(depth + 1)?;
            self.expect_symbol(']')?;
            let ty = Type::Array(count, Box::new(element));
            ty.size_bytes().map_err(|message| error(line, message))?;
            return Ok(ty);
        }
        let word = self.word()?;
        if word == "ptr" {
            if self.eat_word("addrspace") {
                self.expect_symbol('(')?;
                let space = self.natural()?;
                self.expect_symbol(')')?;
                if space != 0 {
                    return Err(error(line, "only address space zero is supported"));
                }
            }
            return Ok(Type::Ptr);
        }
        match word.as_str() {
            "i1" => Ok(Type::Int(1)),
            "i8" => Ok(Type::Int(8)),
            "i16" => Ok(Type::Int(16)),
            "i32" => Ok(Type::Int(32)),
            _ => Err(error(
                line,
                format!("unsupported type '{word}' (use i1/i8/i16/i32, ptr, or arrays)"),
            )),
        }
    }
    fn scalar_ty(&mut self) -> Result<Type, ParseError> {
        let line = self.line();
        let ty = self.ty()?;
        if matches!(ty, Type::Array(..)) {
            Err(error(line, "aggregate SSA values are not supported"))
        } else {
            Ok(ty)
        }
    }
    fn return_ty(&mut self) -> Result<Option<Type>, ParseError> {
        if self.eat_word("void") {
            Ok(None)
        } else {
            self.scalar_ty().map(Some)
        }
    }
    fn operand(&mut self, ty: &Type) -> Result<Operand, ParseError> {
        let line = self.line();
        match self.peek().cloned() {
            Some(Kind::Local(name)) => {
                self.pos += 1;
                Ok(Operand::Local(name))
            }
            Some(Kind::Global(name)) => {
                self.pos += 1;
                if *ty != Type::Ptr {
                    return Err(error(line, "global addresses require ptr type"));
                }
                Ok(Operand::Global(name))
            }
            Some(Kind::Word(word)) => {
                self.pos += 1;
                if word == "undef" || word == "poison" {
                    return Err(error(line, "explicit undef and poison are not supported"));
                }
                if word == "zeroinitializer" {
                    return Ok(if *ty == Type::Ptr {
                        Operand::Null
                    } else {
                        Operand::Constant(0)
                    });
                }
                if *ty == Type::Ptr {
                    if word == "null" {
                        return Ok(Operand::Null);
                    }
                    return Err(error(line, "pointer operands must be locals, globals, or null; constant expressions are unsupported"));
                }
                let Type::Int(width) = ty else {
                    return Err(error(line, "aggregate operand is unsupported"));
                };
                parse_integer(&word, *width)
                    .map(Operand::Constant)
                    .map_err(|message| error(line, message))
            }
            _ => Err(error(line, "expected an operand")),
        }
    }
    fn typed_operand(&mut self) -> Result<(Type, Operand), ParseError> {
        let ty = self.scalar_ty()?;
        let operand = self.operand(&ty)?;
        Ok((ty, operand))
    }
    fn balanced(&mut self, open: char, close: char) -> Result<(), ParseError> {
        self.balanced_nested(open, close, 0)
    }
    fn balanced_nested(&mut self, open: char, close: char, depth: usize) -> Result<(), ParseError> {
        if depth > 128 {
            return self.fail("metadata or attribute nesting exceeds 128 levels");
        }
        self.expect_symbol(open)?;
        while !self.eat_symbol(close) {
            match self.peek() {
                None => return self.fail(format!("unterminated '{open}'")),
                Some(Kind::Symbol('(')) => self.balanced_nested('(', ')', depth + 1)?,
                Some(Kind::Symbol('[')) => self.balanced_nested('[', ']', depth + 1)?,
                Some(Kind::Symbol('{')) => self.balanced_nested('{', '}', depth + 1)?,
                Some(Kind::Symbol('<')) => self.balanced_nested('<', '>', depth + 1)?,
                Some(Kind::Symbol(')' | ']' | '}' | '>')) => {
                    return self.fail("mismatched closing delimiter")
                }
                _ => self.pos += 1,
            }
        }
        Ok(())
    }
    fn metadata_value(&mut self) -> Result<(), ParseError> {
        self.eat_word("distinct");
        match self.peek() {
            Some(Kind::Metadata(_)) => {
                self.pos += 1;
                if self.symbol_is('(') {
                    self.balanced('(', ')')?;
                }
                Ok(())
            }
            Some(Kind::Symbol('!')) => {
                self.pos += 1;
                if self.symbol_is('{') {
                    self.balanced('{', '}')
                } else if matches!(self.peek(), Some(Kind::String(_))) {
                    self.pos += 1;
                    Ok(())
                } else {
                    self.fail("expected metadata tuple or string")
                }
            }
            _ => self.fail("expected a metadata node"),
        }
    }
    fn attachment(&mut self) -> Result<(), ParseError> {
        match self.peek() {
            Some(Kind::Metadata(name)) if !name.bytes().all(|b| b.is_ascii_digit()) => {
                self.pos += 1
            }
            _ => return self.fail("expected a named metadata attachment"),
        }
        self.metadata_value()
    }
    fn metadata_suffix(&mut self) -> Result<(), ParseError> {
        while self.eat_symbol(',') {
            self.attachment()?;
        }
        Ok(())
    }
    fn attribute(&mut self) -> Result<bool, ParseError> {
        if matches!(self.peek(), Some(Kind::String(_))) {
            self.pos += 1;
            if self.eat_symbol('=') {
                self.string()?;
            }
            return Ok(true);
        }
        let Some(Kind::Word(name)) = self.peek() else {
            return Ok(false);
        };
        let name = name.clone();
        if matches!(
            name.as_str(),
            "byval" | "byref" | "inalloca" | "preallocated"
        ) {
            return self.fail(format!("ABI-copy attribute '{name}' is unsupported"));
        }
        if name == "align" {
            self.pos += 1;
            self.alignment()?;
            return Ok(true);
        }
        if matches!(
            name.as_str(),
            "dereferenceable" | "dereferenceable_or_null" | "alignstack"
        ) {
            self.pos += 1;
            self.expect_symbol('(')?;
            self.natural()?;
            self.expect_symbol(')')?;
            return Ok(true);
        }
        if name == "sret" || name == "elementtype" {
            self.pos += 1;
            if self.eat_symbol('(') {
                self.ty()?;
                self.expect_symbol(')')?;
            }
            return Ok(true);
        }
        if matches!(
            name.as_str(),
            "memory"
                | "captures"
                | "allocsize"
                | "vscale_range"
                | "nofpclass"
                | "range"
                | "uwtable"
        ) {
            self.pos += 1;
            if self.symbol_is('(') {
                self.balanced('(', ')')?;
            }
            return Ok(true);
        }
        if matches!(
            name.as_str(),
            "noundef"
                | "zeroext"
                | "signext"
                | "inreg"
                | "noalias"
                | "nocapture"
                | "nonnull"
                | "returned"
                | "readonly"
                | "writeonly"
                | "readnone"
                | "immarg"
                | "nest"
                | "swiftself"
                | "swifterror"
                | "nofree"
                | "nosync"
                | "nounwind"
                | "willreturn"
                | "mustprogress"
                | "norecurse"
                | "speculatable"
                | "noinline"
                | "alwaysinline"
                | "inlinehint"
                | "optnone"
                | "optsize"
                | "minsize"
                | "cold"
                | "hot"
                | "naked"
                | "nobuiltin"
                | "builtin"
                | "noduplicate"
                | "convergent"
                | "null_pointer_is_valid"
                | "sanitize_address"
                | "sanitize_memory"
                | "sanitize_thread"
                | "sanitize_hwaddress"
                | "speculative_load_hardening"
                | "ssp"
                | "sspreq"
                | "sspstrong"
                | "noredzone"
                | "noimplicitfloat"
                | "nonlazybind"
                | "returns_twice"
                | "noreturn"
                | "nocallback"
                | "nocf_check"
                | "nocreateundeforpoison"
                | "skipprofile"
                | "noclone"
                | "nooutline"
                | "disable_sanitizer_instrumentation"
                | "dead_on_unwind"
                | "writable"
                | "allocalign"
                | "allocptr"
                | "strictfp"
                | "shadowcallstack"
                | "optforfuzzing"
                | "fn_ret_thunk_extern"
        ) {
            self.pos += 1;
            return Ok(true);
        }
        Ok(false)
    }
    fn attributes(&mut self) -> Result<(), ParseError> {
        while self.attribute()? {}
        Ok(())
    }
    fn calling_convention(&mut self) -> Result<(), ParseError> {
        if self.eat_word("cc") {
            self.natural()?;
            return Ok(());
        }
        if matches!(self.peek(), Some(Kind::Word(name)) if matches!(name.as_str(),
            "ccc" | "fastcc" | "coldcc" | "tailcc" | "ghccc" | "webkit_jscc" | "anyregcc" |
            "preserve_mostcc" | "preserve_allcc" | "preserve_nonecc" | "cxx_fast_tlscc" |
            "swiftcc" | "swifttailcc" | "cfguard_checkcc"))
        {
            self.pos += 1;
        }
        Ok(())
    }
    fn linkage(&mut self, function: bool, declaration: bool) -> Result<(), ParseError> {
        let mut seen = 0u8;
        while let Some(Kind::Word(name)) = self.peek() {
            let category = match name.as_str() {
                "private"
                | "internal"
                | "available_externally"
                | "linkonce"
                | "weak"
                | "common"
                | "appending"
                | "extern_weak"
                | "linkonce_odr"
                | "weak_odr"
                | "external" => 1,
                "dso_local" | "dso_preemptable" => 2,
                "default" | "hidden" | "protected" => 4,
                "dllimport" | "dllexport" => 8,
                "unnamed_addr" | "local_unnamed_addr" => 16,
                _ => break,
            };
            if seen & category != 0 {
                return self.fail("duplicate or conflicting symbol modifiers");
            }
            if category == 1 {
                if function && matches!(name.as_str(), "common" | "appending") {
                    return self.fail("common/appending linkage is not valid for functions");
                }
                if declaration && !matches!(name.as_str(), "external" | "extern_weak") {
                    return self
                        .fail("function declarations require external or extern_weak linkage");
                }
                if !declaration && name == "extern_weak" {
                    return self.fail("extern_weak symbols cannot have definitions");
                }
            }
            seen |= category;
            self.pos += 1;
        }
        Ok(())
    }
    fn slot(&mut self, name: &str, line: usize) -> Result<(), ParseError> {
        if name.bytes().all(|b| b.is_ascii_digit()) {
            let number = name
                .parse::<usize>()
                .map_err(|_| error(line, "numbered identifier is too large"))?;
            if number != self.next_slot {
                return Err(error(
                    line,
                    format!(
                        "expected numbered local %{}, found %{number}",
                        self.next_slot
                    ),
                ));
            }
            self.next_slot += 1;
        }
        Ok(())
    }
    fn result_name(&mut self, explicit: Option<String>) -> String {
        explicit.unwrap_or_else(|| {
            let name = self.next_slot.to_string();
            self.next_slot += 1;
            name
        })
    }
}

fn parse_integer(text: &str, width: u8) -> Result<u32, String> {
    if text == "true" || text == "false" {
        return if width == 1 {
            Ok(u32::from(text == "true"))
        } else {
            Err("boolean literals require i1".into())
        };
    }
    let (negative, digits) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else {
        (false, text)
    };
    let signed_hex = !negative && digits.starts_with("s0x");
    let (radix, digits) = if !negative {
        if let Some(rest) = digits
            .strip_prefix("u0x")
            .or_else(|| digits.strip_prefix("s0x"))
        {
            (16u32, rest)
        } else {
            (10u32, digits)
        }
    } else {
        (10u32, digits)
    };
    if digits.is_empty() {
        return Err(format!("invalid integer literal '{text}'"));
    }
    let mut value = 0u32;
    for digit in digits.chars() {
        let digit = digit
            .to_digit(radix)
            .ok_or_else(|| format!("invalid integer literal '{text}'"))?;
        value = value.wrapping_mul(radix).wrapping_add(digit);
    }
    if signed_hex {
        // LLVM's signed hexadecimal token uses its active bit width, then
        // sign-extends that bit vector to the instruction's integer width.
        let significant = digits.trim_start_matches('0');
        if !significant.is_empty() && significant.len() <= 8 {
            let leading = significant.as_bytes()[0] as char;
            let active = (significant.len() as u32 - 1) * 4
                + (32 - leading.to_digit(16).unwrap().leading_zeros());
            if active < 32 {
                value |= u32::MAX << active;
            }
        }
    }
    if negative {
        value = value.wrapping_neg();
    }
    if width < 32 {
        value &= (1u32 << width) - 1;
    }
    Ok(value)
}

impl Parser {
    fn function(&mut self, defined: bool) -> Result<Option<Function>, ParseError> {
        let line = self.line();
        self.pos += 1;
        self.linkage(true, !defined)?;
        self.calling_convention()?;
        self.attributes()?;
        let return_type = self.return_ty()?;
        let name = self.global()?;
        if self.global_names.contains(&name) {
            return Err(error(line, format!("duplicate global symbol @{name}")));
        }
        if let Some(arity) = debug_arity(&name) {
            if defined {
                return Err(error(
                    line,
                    "debug intrinsics cannot have executable definitions",
                ));
            }
            if !self.debug_declarations.insert(name) {
                return Err(error(line, "duplicate debug intrinsic declaration"));
            }
            self.expect_symbol('(')?;
            for index in 0..arity {
                if index != 0 {
                    self.expect_symbol(',')?;
                }
                self.expect_word("metadata")?;
                if matches!(self.peek(), Some(Kind::Local(_))) {
                    self.pos += 1;
                }
            }
            self.expect_symbol(')')?;
            if return_type.is_some() {
                return Err(error(line, "debug intrinsics must return void"));
            }
            self.call_attributes()?;
            return Ok(None);
        }
        self.expect_symbol('(')?;
        self.next_slot = 0;
        let mut parameters = Vec::new();
        let mut parameter_names = HashSet::new();
        let mut variadic = false;
        if !self.eat_symbol(')') {
            loop {
                if self.eat_word("...") {
                    variadic = true;
                    self.expect_symbol(')')?;
                    break;
                }
                let parameter_line = self.line();
                let ty = self.scalar_ty()?;
                self.attributes()?;
                let parameter = if matches!(self.peek(), Some(Kind::Local(_))) {
                    let name = self.local()?;
                    self.slot(&name, parameter_line)?;
                    name
                } else {
                    self.result_name(None)
                };
                if !parameter_names.insert(parameter.clone()) {
                    return Err(error(
                        parameter_line,
                        format!("duplicate parameter %{parameter}"),
                    ));
                }
                parameters.push((parameter, ty));
                if self.eat_symbol(')') {
                    break;
                }
                self.expect_symbol(',')?;
            }
        }
        if variadic && defined {
            return Err(error(line, "variadic function definitions are unsupported"));
        }
        loop {
            if self.attribute()? {
                continue;
            }
            if matches!(self.peek(), Some(Kind::Group(_))) {
                self.pos += 1;
                continue;
            }
            if self.eat_word("unnamed_addr") || self.eat_word("local_unnamed_addr") {
                continue;
            }
            if self.eat_word("section") || self.eat_word("partition") {
                self.string()?;
                continue;
            }
            if self.eat_word("comdat") {
                if self.symbol_is('(') {
                    self.balanced('(', ')')?;
                }
                continue;
            }
            if matches!(self.peek(), Some(Kind::Metadata(_))) {
                self.attachment()?;
                continue;
            }
            break;
        }
        let signature = Signature {
            return_type: return_type.clone(),
            parameters: parameters.iter().map(|(_, ty)| ty.clone()).collect(),
            variadic,
            defined,
        };
        if let Some(previous) = self.signatures.get(&name) {
            if previous.return_type != signature.return_type
                || previous.parameters != signature.parameters
                || previous.variadic != signature.variadic
            {
                return Err(error(line, format!("conflicting signatures for @{name}")));
            }
            if previous.defined || !defined {
                return Err(error(line, format!("duplicate function @{name}")));
            }
        }
        self.signatures.insert(name.clone(), signature);
        if !defined {
            return Ok(None);
        }
        self.expect_symbol('{')?;
        let mut blocks = Vec::new();
        let mut block_lines = Vec::new();
        let mut instruction_lines = Vec::new();
        while !self.eat_symbol('}') {
            if self.peek().is_none() {
                return self.fail("unterminated function body");
            }
            let block_line = self.line();
            let label = if self.tokens.get(self.pos + 1).map(|token| &token.kind)
                == Some(&Kind::Symbol(':'))
            {
                let label = match self.peek() {
                    Some(Kind::Word(name)) => block_name(name, block_line)?,
                    Some(Kind::String(name)) => quoted_name(name.clone(), block_line)?,
                    _ => return self.fail("invalid block label"),
                };
                self.pos += 2;
                self.slot(&label, block_line)?;
                label
            } else if blocks.is_empty() {
                self.result_name(None)
            } else {
                return self.fail("expected a basic block label");
            };
            let mut instructions = Vec::new();
            let mut locations = Vec::new();
            while !self.symbol_is('}')
                && self.peek().is_some()
                && self.tokens.get(self.pos + 1).map(|token| &token.kind)
                    != Some(&Kind::Symbol(':'))
            {
                let instruction_line = self.line();
                if self.debug_record()? || self.debug_call()? {
                    continue;
                }
                let instruction = self.instruction()?;
                locations.push(instruction_line);
                instructions.push(instruction);
            }
            blocks.push(Block {
                label,
                instructions,
            });
            block_lines.push(block_line);
            instruction_lines.push(locations);
        }
        if blocks.is_empty() {
            return Err(error(line, "a function must contain a basic block"));
        }
        self.lines.push(FunctionLines {
            line,
            blocks: block_lines,
            instructions: instruction_lines,
        });
        Ok(Some(Function {
            name,
            return_type,
            parameters,
            blocks,
        }))
    }
    fn call_attributes(&mut self) -> Result<(), ParseError> {
        loop {
            if self.attribute()? {
                continue;
            }
            if matches!(self.peek(), Some(Kind::Group(_))) {
                self.pos += 1;
                continue;
            }
            return Ok(());
        }
    }
    fn debug_argument(&mut self) -> Result<(), ParseError> {
        if matches!(self.peek(), Some(Kind::Metadata(_) | Kind::Symbol('!'))) {
            self.metadata_value()
        } else {
            self.typed_operand().map(|_| ())
        }
    }
    fn debug_record(&mut self) -> Result<bool, ParseError> {
        let Some(Kind::Debug(name)) = self.peek() else {
            return Ok(false);
        };
        let arity = debug_record_arity(name).unwrap();
        self.pos += 1;
        self.expect_symbol('(')?;
        for index in 0..arity {
            if index != 0 {
                self.expect_symbol(',')?;
            }
            if (index == 0 && arity != 2) || (index == 4 && arity == 7) {
                self.debug_argument()?;
            } else {
                self.metadata_value()?;
            }
        }
        self.expect_symbol(')')?;
        Ok(true)
    }
    fn debug_call(&mut self) -> Result<bool, ParseError> {
        if !self.word_is("call")
            || !matches!(self.tokens.get(self.pos + 1).map(|token| &token.kind), Some(Kind::Word(word)) if word == "void")
        {
            return Ok(false);
        }
        let Some(Kind::Global(name)) = self.tokens.get(self.pos + 2).map(|token| &token.kind)
        else {
            return Ok(false);
        };
        let Some(arity) = debug_arity(name) else {
            return Ok(false);
        };
        self.pos += 3;
        self.expect_symbol('(')?;
        for index in 0..arity {
            if index != 0 {
                self.expect_symbol(',')?;
            }
            self.expect_word("metadata")?;
            if (index == 0 && arity != 1) || (index == 4 && arity == 6) {
                self.debug_argument()?;
            } else {
                self.metadata_value()?;
            }
        }
        self.expect_symbol(')')?;
        self.call_attributes()?;
        self.metadata_suffix()?;
        Ok(true)
    }
    fn instruction(&mut self) -> Result<Instruction, ParseError> {
        let line = self.line();
        let explicit = if matches!(self.peek(), Some(Kind::Local(_))) {
            let name = self.local()?;
            self.expect_symbol('=')?;
            self.slot(&name, line)?;
            Some(name)
        } else {
            None
        };
        let opcode_line = self.line();
        let mut opcode = self.word()?;
        if matches!(opcode.as_str(), "tail" | "musttail" | "notail") {
            opcode = self.word()?;
            if opcode != "call" {
                return Err(error(line, "tail markers must precede call"));
            }
        }
        let binary = match opcode.as_str() {
            "add" => Some(BinaryOp::Add),
            "sub" => Some(BinaryOp::Sub),
            "mul" => Some(BinaryOp::Mul),
            "udiv" => Some(BinaryOp::UDiv),
            "sdiv" => Some(BinaryOp::SDiv),
            "urem" => Some(BinaryOp::URem),
            "srem" => Some(BinaryOp::SRem),
            "and" => Some(BinaryOp::And),
            "or" => Some(BinaryOp::Or),
            "xor" => Some(BinaryOp::Xor),
            "shl" => Some(BinaryOp::Shl),
            "lshr" => Some(BinaryOp::LShr),
            "ashr" => Some(BinaryOp::AShr),
            _ => None,
        };
        let cast = match opcode.as_str() {
            "trunc" => Some(CastOp::Trunc),
            "zext" => Some(CastOp::ZExt),
            "sext" => Some(CastOp::SExt),
            "bitcast" => Some(CastOp::Bitcast),
            "ptrtoint" => Some(CastOp::PtrToInt),
            "inttoptr" => Some(CastOp::IntToPtr),
            _ => None,
        };
        let instruction = if let Some(op) = binary {
            let mut seen = HashSet::new();
            while matches!(self.peek(), Some(Kind::Word(flag)) if matches!(flag.as_str(), "nuw" | "nsw" | "exact" | "disjoint"))
            {
                let flag = self.word()?;
                let valid = if flag == "exact" {
                    matches!(
                        op,
                        BinaryOp::UDiv | BinaryOp::SDiv | BinaryOp::LShr | BinaryOp::AShr
                    )
                } else if flag == "disjoint" {
                    op == BinaryOp::Or
                } else {
                    matches!(
                        op,
                        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Shl
                    )
                };
                if !valid || !seen.insert(flag.clone()) {
                    return Err(error(
                        line,
                        format!("invalid or duplicate '{flag}' flag on {opcode}"),
                    ));
                }
            }
            let ty = self.scalar_ty()?;
            if !matches!(ty, Type::Int(_)) {
                return Err(error(
                    line,
                    "integer binary operations require integer type",
                ));
            }
            let lhs = self.operand(&ty)?;
            self.expect_symbol(',')?;
            let rhs = self.operand(&ty)?;
            Instruction::Binary {
                dst: self.result_name(explicit),
                op,
                ty,
                lhs,
                rhs,
            }
        } else if let Some(op) = cast {
            let mut seen = HashSet::new();
            while matches!(self.peek(), Some(Kind::Word(flag)) if matches!(flag.as_str(), "nuw" | "nsw" | "nneg"))
            {
                let flag = self.word()?;
                let valid = if flag == "nneg" {
                    op == CastOp::ZExt
                } else {
                    op == CastOp::Trunc
                };
                if !valid || !seen.insert(flag.clone()) {
                    return Err(error(
                        line,
                        format!("invalid or duplicate '{flag}' flag on {opcode}"),
                    ));
                }
            }
            let (from, value) = self.typed_operand()?;
            self.expect_word("to")?;
            let to = self.scalar_ty()?;
            validate_cast(op, &from, &to).map_err(|message| error(line, message))?;
            Instruction::Cast {
                dst: self.result_name(explicit),
                op,
                from,
                value,
                to,
            }
        } else {
            match opcode.as_str() {
                "icmp" => {
                    self.eat_word("samesign");
                    let predicate = match self.word()?.as_str() {
                        "eq" => Predicate::Eq,
                        "ne" => Predicate::Ne,
                        "ult" => Predicate::Ult,
                        "ule" => Predicate::Ule,
                        "ugt" => Predicate::Ugt,
                        "uge" => Predicate::Uge,
                        "slt" => Predicate::Slt,
                        "sle" => Predicate::Sle,
                        "sgt" => Predicate::Sgt,
                        "sge" => Predicate::Sge,
                        _ => return Err(error(line, "invalid integer comparison predicate")),
                    };
                    let ty = self.scalar_ty()?;
                    let lhs = self.operand(&ty)?;
                    self.expect_symbol(',')?;
                    let rhs = self.operand(&ty)?;
                    Instruction::ICmp {
                        dst: self.result_name(explicit),
                        predicate,
                        ty,
                        lhs,
                        rhs,
                    }
                }
                "select" => {
                    let (condition_type, condition) = self.typed_operand()?;
                    if condition_type != Type::Int(1) {
                        return Err(error(line, "select condition must be i1"));
                    }
                    self.expect_symbol(',')?;
                    let (ty, on_true) = self.typed_operand()?;
                    self.expect_symbol(',')?;
                    let (false_type, on_false) = self.typed_operand()?;
                    if false_type != ty {
                        return Err(error(line, "select alternatives have different types"));
                    }
                    Instruction::Select {
                        dst: self.result_name(explicit),
                        condition,
                        ty,
                        on_true,
                        on_false,
                    }
                }
                "phi" => {
                    let ty = self.scalar_ty()?;
                    let mut incoming = Vec::new();
                    loop {
                        self.expect_symbol('[')?;
                        let value = self.operand(&ty)?;
                        self.expect_symbol(',')?;
                        let predecessor = self.local()?;
                        self.expect_symbol(']')?;
                        incoming.push((value, predecessor));
                        if self.symbol_is(',')
                            && self.tokens.get(self.pos + 1).map(|token| &token.kind)
                                == Some(&Kind::Symbol('['))
                        {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    Instruction::Phi {
                        dst: self.result_name(explicit),
                        ty,
                        incoming,
                    }
                }
                "alloca" => {
                    let ty = self.ty()?;
                    let mut count = 1;
                    let mut align = ty.alignment();
                    let mut has_count = false;
                    let mut has_align = false;
                    let mut has_space = false;
                    while self.symbol_is(',')
                        && !matches!(
                            self.tokens.get(self.pos + 1).map(|token| &token.kind),
                            Some(Kind::Metadata(_))
                        )
                    {
                        self.pos += 1;
                        if self.eat_word("align") {
                            if has_align {
                                return self.fail("duplicate alloca alignment");
                            }
                            align = self.alignment()?;
                            has_align = true;
                        } else if self.eat_word("addrspace") {
                            if has_space {
                                return self.fail("duplicate alloca address space");
                            }
                            self.expect_symbol('(')?;
                            if self.natural()? != 0 {
                                return self.fail("only address space zero is supported");
                            }
                            self.expect_symbol(')')?;
                            has_space = true;
                        } else {
                            if has_count || has_align || has_space {
                                return self.fail("unexpected alloca count");
                            }
                            let (count_type, operand) = self.typed_operand()?;
                            if !matches!(count_type, Type::Int(_)) {
                                return Err(error(line, "alloca count must be an integer"));
                            }
                            let Operand::Constant(value) = operand else {
                                return Err(error(line, "dynamic alloca is unsupported"));
                            };
                            count = value as usize;
                            has_count = true;
                        }
                    }
                    ty.size_bytes()
                        .and_then(|size| {
                            size.checked_mul(count)
                                .ok_or_else(|| "alloca size overflows".into())
                        })
                        .map_err(|message| error(line, message))?;
                    Instruction::Alloca {
                        dst: self.result_name(explicit),
                        ty,
                        count,
                        align,
                    }
                }
                "load" => {
                    if self.word_is("atomic") || self.word_is("volatile") {
                        return self.fail("atomic and volatile memory operations are unsupported");
                    }
                    let ty = self.scalar_ty()?;
                    self.expect_symbol(',')?;
                    let (pointer_type, pointer) = self.typed_operand()?;
                    if pointer_type != Type::Ptr {
                        return Err(error(line, "load address must have ptr type"));
                    }
                    let align = self.memory_alignment(ty.alignment())?;
                    Instruction::Load {
                        dst: self.result_name(explicit),
                        ty,
                        pointer,
                        align,
                    }
                }
                "store" => {
                    if explicit.is_some() {
                        return Err(error(line, "store has no SSA result"));
                    }
                    if self.word_is("atomic") || self.word_is("volatile") {
                        return self.fail("atomic and volatile memory operations are unsupported");
                    }
                    let (ty, value) = self.typed_operand()?;
                    self.expect_symbol(',')?;
                    let (pointer_type, pointer) = self.typed_operand()?;
                    if pointer_type != Type::Ptr {
                        return Err(error(line, "store address must have ptr type"));
                    }
                    let align = self.memory_alignment(ty.alignment())?;
                    Instruction::Store {
                        ty,
                        value,
                        pointer,
                        align,
                    }
                }
                "getelementptr" => {
                    let mut flags = HashSet::new();
                    while matches!(self.peek(), Some(Kind::Word(flag)) if matches!(flag.as_str(), "inbounds" | "nuw" | "nusw"))
                    {
                        let flag = self.word()?;
                        if !flags.insert(flag) {
                            return self.fail("duplicate getelementptr flag");
                        }
                    }
                    let source_type = self.ty()?;
                    self.expect_symbol(',')?;
                    let (base_type, base) = self.typed_operand()?;
                    if base_type != Type::Ptr {
                        return Err(error(line, "getelementptr base must have ptr type"));
                    }
                    let mut indices = Vec::new();
                    let mut indexed_type = &source_type;
                    while self.symbol_is(',')
                        && !matches!(
                            self.tokens.get(self.pos + 1).map(|token| &token.kind),
                            Some(Kind::Metadata(_))
                        )
                    {
                        self.pos += 1;
                        let (ty, index) = self.typed_operand()?;
                        if !matches!(ty, Type::Int(_)) {
                            return Err(error(line, "getelementptr indices must be integers"));
                        }
                        if !indices.is_empty() {
                            let Type::Array(_, element) = indexed_type else {
                                return Err(error(
                                    line,
                                    "getelementptr indexes through a non-array type",
                                ));
                            };
                            indexed_type = element;
                        }
                        indices.push((ty, index));
                    }
                    Instruction::Gep {
                        dst: self.result_name(explicit),
                        source_type,
                        base,
                        indices,
                    }
                }
                "call" => {
                    self.calling_convention()?;
                    self.attributes()?;
                    let return_type = self.return_ty()?;
                    if self.symbol_is('(') {
                        return self
                            .fail("variadic or explicitly typed call signatures are unsupported");
                    }
                    let function = self.global()?;
                    if debug_arity(&function).is_some() {
                        return Err(error(
                            line,
                            "malformed debug intrinsic call; expected void and metadata arguments",
                        ));
                    }
                    self.expect_symbol('(')?;
                    let mut args = Vec::new();
                    if !self.eat_symbol(')') {
                        loop {
                            let ty = self.scalar_ty()?;
                            self.attributes()?;
                            let value = self.operand(&ty)?;
                            args.push((ty, value));
                            if self.eat_symbol(')') {
                                break;
                            }
                            self.expect_symbol(',')?;
                        }
                    }
                    self.call_attributes()?;
                    let dst = if return_type.is_some() {
                        Some(self.result_name(explicit))
                    } else if explicit.is_some() {
                        return Err(error(line, "void call has no SSA result"));
                    } else {
                        None
                    };
                    Instruction::Call {
                        dst,
                        function,
                        args,
                        return_type,
                    }
                }
                "br" => {
                    if explicit.is_some() {
                        return Err(error(line, "branch has no SSA result"));
                    }
                    if self.eat_word("label") {
                        Instruction::Br {
                            condition: None,
                            on_true: self.local()?,
                            on_false: None,
                        }
                    } else {
                        let (ty, condition) = self.typed_operand()?;
                        if ty != Type::Int(1) {
                            return Err(error(line, "branch condition must be i1"));
                        }
                        self.expect_symbol(',')?;
                        self.expect_word("label")?;
                        let on_true = self.local()?;
                        self.expect_symbol(',')?;
                        self.expect_word("label")?;
                        let on_false = self.local()?;
                        Instruction::Br {
                            condition: Some(condition),
                            on_true,
                            on_false: Some(on_false),
                        }
                    }
                }
                "ret" => {
                    if explicit.is_some() {
                        return Err(error(line, "return has no SSA result"));
                    }
                    if self.eat_word("void") {
                        Instruction::Ret {
                            ty: None,
                            value: None,
                        }
                    } else {
                        let (ty, value) = self.typed_operand()?;
                        Instruction::Ret {
                            ty: Some(ty),
                            value: Some(value),
                        }
                    }
                }
                "unreachable" => {
                    if explicit.is_some() {
                        return Err(error(line, "unreachable has no SSA result"));
                    }
                    Instruction::Unreachable
                }
                _ => {
                    return Err(error(
                        opcode_line,
                        format!("unsupported instruction '{opcode}'"),
                    ))
                }
            }
        };
        self.metadata_suffix()?;
        Ok(instruction)
    }
    fn memory_alignment(&mut self, default: usize) -> Result<usize, ParseError> {
        if self.symbol_is(',')
            && matches!(self.tokens.get(self.pos + 1).map(|token| &token.kind), Some(Kind::Word(word)) if word == "align")
        {
            self.pos += 2;
            self.alignment()
        } else {
            Ok(default)
        }
    }
    fn initializer(&mut self, ty: &Type, bytes: &mut Vec<u8>) -> Result<(), ParseError> {
        let line = self.line();
        if self.eat_word("zeroinitializer") {
            let size = ty.size_bytes().map_err(|message| error(line, message))?;
            let end = bytes
                .len()
                .checked_add(size)
                .ok_or_else(|| error(line, "global initializer size overflows"))?;
            bytes
                .try_reserve(size)
                .map_err(|_| error(line, "global initializer is too large"))?;
            bytes.resize(end, 0);
            return Ok(());
        }
        match ty {
            Type::Array(count, element) => {
                if self.eat_word("c") {
                    if **element != Type::Int(8) {
                        return Err(error(line, "c-string initializer requires an i8 array"));
                    }
                    let value = self.string()?;
                    if value.len() != *count {
                        return Err(error(line, "c-string length does not match array type"));
                    }
                    bytes.extend(value);
                } else {
                    self.expect_symbol('[')?;
                    for index in 0..*count {
                        if index != 0 {
                            self.expect_symbol(',')?;
                        }
                        let element_line = self.line();
                        let actual = self.ty()?;
                        if actual != **element {
                            return Err(error(
                                element_line,
                                "array initializer element type mismatch",
                            ));
                        }
                        self.initializer(element, bytes)?;
                    }
                    self.expect_symbol(']')?;
                }
            }
            Type::Int(_) | Type::Ptr => {
                let operand = self.operand(ty)?;
                let value = match operand {
                    Operand::Constant(value) => value,
                    Operand::Null => 0,
                    _ => return Err(error(line, "global initializers must be literal values; address relocations are unsupported")),
                };
                let size = ty.size_bytes().map_err(|message| error(line, message))?;
                bytes.extend_from_slice(&value.to_le_bytes()[..size]);
            }
        }
        Ok(())
    }
    fn global_definition(&mut self) -> Result<Global, ParseError> {
        let line = self.line();
        let name = self.global()?;
        if debug_arity(&name).is_some() {
            return Err(error(
                line,
                "reserved debug intrinsic name cannot be a data global",
            ));
        }
        if self.signatures.contains_key(&name) || !self.global_names.insert(name.clone()) {
            return Err(error(line, format!("duplicate global symbol @{name}")));
        }
        self.expect_symbol('=')?;
        self.linkage(false, false)?;
        if self.word_is("thread_local") {
            return self.fail("thread-local globals are unsupported");
        }
        if self.eat_word("addrspace") {
            self.expect_symbol('(')?;
            if self.natural()? != 0 {
                return self.fail("only address space zero is supported");
            }
            self.expect_symbol(')')?;
        }
        self.eat_word("externally_initialized");
        if !self.eat_word("global") && !self.eat_word("constant") {
            return self.fail("expected a global or constant definition; aliases and external globals are unsupported");
        }
        let ty = self.ty()?;
        let mut initializer = Vec::new();
        self.initializer(&ty, &mut initializer)?;
        let mut align = ty.alignment();
        let mut explicit_align = false;
        while self.eat_symbol(',') {
            if self.eat_word("align") {
                if explicit_align {
                    return self.fail("duplicate global alignment");
                }
                align = self.alignment()?;
                explicit_align = true;
            } else if self.eat_word("section") || self.eat_word("partition") {
                self.string()?;
            } else if self.eat_word("comdat") {
                if self.symbol_is('(') {
                    self.balanced('(', ')')?;
                }
            } else {
                self.attachment()?;
            }
        }
        Ok(Global {
            name,
            ty,
            initializer,
            align,
        })
    }
}

fn validate_cast(op: CastOp, from: &Type, to: &Type) -> Result<(), String> {
    let valid = match op {
        CastOp::Trunc => matches!((from, to), (Type::Int(a), Type::Int(b)) if a > b),
        CastOp::ZExt | CastOp::SExt => matches!((from, to), (Type::Int(a), Type::Int(b)) if a < b),
        CastOp::Bitcast => from == to && !matches!(from, Type::Array(..)),
        CastOp::PtrToInt => *from == Type::Ptr && matches!(to, Type::Int(_)),
        CastOp::IntToPtr => matches!(from, Type::Int(_)) && *to == Type::Ptr,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("invalid {op:?} cast from {from:?} to {to:?}"))
    }
}

fn validate_layout(layout: &[u8], line: usize) -> Result<(), ParseError> {
    let layout = std::str::from_utf8(layout).map_err(|_| error(line, "datalayout is not UTF-8"))?;
    if layout.is_empty() {
        return Ok(());
    }
    let mut seen = HashSet::new();
    let number = |field: &str| -> Result<u32, ParseError> {
        if field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(error(line, "malformed numeric datalayout field"));
        }
        field
            .parse::<u32>()
            .map_err(|_| error(line, "datalayout field is too large"))
    };
    let alignment = |field: &str, zero_allowed: bool| -> Result<u32, ParseError> {
        let value = number(field)?;
        if (value == 0 && zero_allowed) || (value >= 8 && value.is_power_of_two()) {
            Ok(value)
        } else {
            Err(error(
                line,
                "datalayout alignment must be a power of two in bits",
            ))
        }
    };
    for part in layout.split('-') {
        let key;
        if part == "E" {
            return Err(error(line, "big-endian datalayout is unsupported"));
        }
        if part == "e" {
            key = "endian".to_owned();
        } else if let Some(pointer) = part.strip_prefix('p') {
            let fields: Vec<_> = pointer.split(':').collect();
            if !(3..=5).contains(&fields.len()) {
                return Err(error(line, "malformed pointer datalayout"));
            }
            let space = if fields[0].is_empty() {
                0
            } else {
                number(fields[0])?
            };
            let width = number(fields[1])?;
            let abi = alignment(fields[2], false)?;
            if width == 0 {
                return Err(error(line, "zero pointer width"));
            }
            if fields.len() >= 4 {
                alignment(fields[3], false)?;
            }
            let index_width = if fields.len() == 5 {
                number(fields[4])?
            } else {
                width
            };
            if index_width == 0 || index_width > width {
                return Err(error(line, "invalid pointer index width"));
            }
            if space == 0 && (width != 32 || abi != 32 || index_width != 32) {
                return Err(error(
                    line,
                    "default pointers require 32-bit size, ABI alignment and index width",
                ));
            }
            key = format!("p{space}");
        } else if part.starts_with(['i', 'f', 'v', 's']) {
            let kind = part.as_bytes()[0] as char;
            let fields: Vec<_> = part[1..].split(':').collect();
            if !(2..=3).contains(&fields.len()) {
                return Err(error(line, "malformed scalar datalayout"));
            }
            let width = number(fields[0])?;
            if width == 0 {
                return Err(error(line, "zero scalar width in datalayout"));
            }
            let abi = alignment(fields[1], false)?;
            if fields.len() == 3 {
                alignment(fields[2], false)?;
            }
            if kind == 'i' {
                let natural = match width {
                    1 | 8 => Some(8),
                    16 => Some(16),
                    32 => Some(32),
                    _ => None,
                };
                if natural.is_some_and(|natural| abi != natural) {
                    return Err(error(
                        line,
                        "custom ABI alignment for supported integers is unsupported",
                    ));
                }
            }
            key = format!("{kind}{width}");
        } else if let Some(aggregate) = part.strip_prefix("a:") {
            let fields: Vec<_> = aggregate.split(':').collect();
            if !(1..=2).contains(&fields.len()) {
                return Err(error(line, "malformed aggregate datalayout"));
            }
            alignment(fields[0], true)?;
            if fields.len() == 2 {
                alignment(fields[1], false)?;
            }
            key = "aggregate".to_owned();
        } else if let Some(mangling) = part.strip_prefix("m:") {
            if !matches!(mangling, "e" | "l" | "m" | "o" | "w" | "a" | "x") {
                return Err(error(line, "unsupported mangling datalayout"));
            }
            key = "mangling".to_owned();
        } else if let Some(non_integral) = part.strip_prefix("ni:") {
            for space in non_integral.split(':') {
                if number(space)? == 0 {
                    return Err(error(line, "default pointers cannot be non-integral"));
                }
            }
            key = "non-integral".to_owned();
        } else if let Some(native) = part.strip_prefix('n') {
            for width in native.split(':') {
                if number(width)? == 0 {
                    return Err(error(line, "zero native integer width"));
                }
            }
            key = "native".to_owned();
        } else if let Some(stack) = part.strip_prefix('S') {
            alignment(stack, false)?;
            key = "stack".to_owned();
        } else if part.starts_with(['A', 'P', 'G']) {
            let space = number(&part[1..])?;
            if space != 0 {
                return Err(error(
                    line,
                    "nonzero default allocation/program/global address spaces are unsupported",
                ));
            }
            key = part[..1].to_owned();
        } else if let Some(function) = part.strip_prefix('F') {
            let Some(value) = function
                .strip_prefix('i')
                .or_else(|| function.strip_prefix('n'))
            else {
                return Err(error(line, "malformed function pointer alignment"));
            };
            alignment(value, false)?;
            key = "function".to_owned();
        } else {
            return Err(error(
                line,
                format!("unsupported or malformed datalayout component '{part}'"),
            ));
        }
        if !seen.insert(key) {
            return Err(error(line, "duplicate datalayout component"));
        }
    }
    Ok(())
}

/// Parse and validate textual LLVM IR, with source-line diagnostics.
pub fn parse_module(source: &str) -> Result<Module, ParseError> {
    let mut parser = Parser {
        tokens: lex(source)?,
        pos: 0,
        eof_line: source.bytes().filter(|b| *b == b'\n').count() + 1,
        signatures: HashMap::new(),
        global_names: HashSet::new(),
        debug_declarations: HashSet::new(),
        lines: Vec::new(),
        next_slot: 0,
    };
    let mut module = Module {
        functions: Vec::new(),
        globals: Vec::new(),
    };
    let mut bookkeeping = HashSet::new();
    while parser.peek().is_some() {
        let line = parser.line();
        if parser.word_is("define") {
            if let Some(function) = parser.function(true)? {
                module.functions.push(function);
            }
        } else if parser.word_is("declare") {
            parser.function(false)?;
        } else if matches!(parser.peek(), Some(Kind::Global(_))) {
            module.globals.push(parser.global_definition()?);
        } else if parser.eat_word("source_filename") {
            if !bookkeeping.insert("source_filename".to_owned()) {
                return Err(error(line, "duplicate source_filename"));
            }
            parser.expect_symbol('=')?;
            parser.string()?;
        } else if parser.eat_word("target") {
            let target = parser.word()?;
            if !matches!(target.as_str(), "triple" | "datalayout") {
                return Err(error(line, "expected target triple or datalayout"));
            }
            if !bookkeeping.insert(target.clone()) {
                return Err(error(line, format!("duplicate target {target}")));
            }
            parser.expect_symbol('=')?;
            let value = parser.string()?;
            if target == "datalayout" {
                validate_layout(&value, line)?;
            }
        } else if parser.eat_word("attributes") {
            let Some(Kind::Group(name)) = parser.peek().cloned() else {
                return parser.fail("expected an attribute group identifier");
            };
            if !name.bytes().all(|b| b.is_ascii_digit()) {
                return parser.fail("attribute group identifier must be numeric");
            }
            if !bookkeeping.insert(format!("#{name}")) {
                return Err(error(line, "duplicate attribute group"));
            }
            parser.pos += 1;
            parser.expect_symbol('=')?;
            parser.expect_symbol('{')?;
            while !parser.eat_symbol('}') {
                if !parser.attribute()? {
                    return parser.fail("unsupported or malformed attribute group member");
                }
            }
        } else if let Some(Kind::Metadata(name)) = parser.peek().cloned() {
            if !bookkeeping.insert(format!("!{name}")) {
                return Err(error(line, "duplicate metadata definition"));
            }
            parser.pos += 1;
            parser.expect_symbol('=')?;
            parser.metadata_value()?;
        } else if matches!(parser.peek(), Some(Kind::Word(name)) if name.starts_with('$')) {
            let name = parser.word()?;
            if !bookkeeping.insert(name) {
                return Err(error(line, "duplicate comdat definition"));
            }
            parser.expect_symbol('=')?;
            parser.expect_word("comdat")?;
            let selection = parser.word()?;
            if !matches!(
                selection.as_str(),
                "any" | "exactmatch" | "largest" | "noduplicates" | "samesize"
            ) {
                return Err(error(line, "invalid comdat selection kind"));
            }
        } else {
            return Err(error(
                line,
                "unsupported or malformed top-level LLVM construct",
            ));
        }
    }
    for token in &parser.tokens {
        if let Kind::Group(name) = &token.kind {
            if !bookkeeping.contains(&format!("#{name}")) {
                return Err(error(
                    token.line,
                    format!("undefined attribute group #{name}"),
                ));
            }
        }
    }
    for (function, lines) in module.functions.iter().zip(&parser.lines) {
        validate_function(function, lines, &module, &parser.signatures)?;
    }
    Ok(module)
}

#[derive(Clone)]
struct Definition {
    ty: Type,
    block: Option<usize>,
    position: usize,
}

fn result(instruction: &Instruction) -> Option<(&str, Type)> {
    match instruction {
        Instruction::Binary { dst, ty, .. }
        | Instruction::Select { dst, ty, .. }
        | Instruction::Phi { dst, ty, .. }
        | Instruction::Load { dst, ty, .. } => Some((dst, ty.clone())),
        Instruction::ICmp { dst, .. } => Some((dst, Type::Int(1))),
        Instruction::Cast { dst, to, .. } => Some((dst, to.clone())),
        Instruction::Alloca { dst, .. } | Instruction::Gep { dst, .. } => Some((dst, Type::Ptr)),
        Instruction::Call {
            dst: Some(dst),
            return_type: Some(ty),
            ..
        } => Some((dst, ty.clone())),
        _ => None,
    }
}

fn terminator(instruction: &Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Br { .. } | Instruction::Ret { .. } | Instruction::Unreachable
    )
}

struct UseSite {
    block: usize,
    position: usize,
    line: usize,
}

struct Validation<'a> {
    definitions: HashMap<&'a str, Definition>,
    globals: HashSet<&'a str>,
    dominators: Vec<HashSet<usize>>,
    reachable: Vec<bool>,
}
impl Validation<'_> {
    fn operand(&self, operand: &Operand, ty: &Type, site: &UseSite) -> Result<(), ParseError> {
        match operand {
            Operand::Local(name) => {
                let definition = self
                    .definitions
                    .get(name.as_str())
                    .ok_or_else(|| error(site.line, format!("undefined SSA value %{name}")))?;
                if definition.ty != *ty {
                    return Err(error(
                        site.line,
                        format!(
                            "type mismatch for %{name}: defined as {:?}, used as {ty:?}",
                            definition.ty
                        ),
                    ));
                }
                if let Some(block) = definition.block {
                    if block == site.block {
                        if definition.position >= site.position {
                            return Err(error(
                                site.line,
                                format!("SSA value %{name} does not dominate its use"),
                            ));
                        }
                    } else if self.reachable[site.block]
                        && !self.dominators[site.block].contains(&block)
                    {
                        return Err(error(
                            site.line,
                            format!("SSA value %{name} does not dominate its use"),
                        ));
                    }
                }
            }
            Operand::Global(name) => {
                if *ty != Type::Ptr || !self.globals.contains(name.as_str()) {
                    return Err(error(
                        site.line,
                        format!("unknown or incorrectly typed global @{name}"),
                    ));
                }
            }
            Operand::Null => {
                if *ty != Type::Ptr {
                    return Err(error(site.line, "null requires pointer type"));
                }
            }
            Operand::Constant(_) => {
                if !matches!(ty, Type::Int(_)) {
                    return Err(error(site.line, "integer literal requires integer type"));
                }
            }
        }
        Ok(())
    }
}

fn validate_function(
    function: &Function,
    lines: &FunctionLines,
    module: &Module,
    signatures: &HashMap<String, Signature>,
) -> Result<(), ParseError> {
    let mut definitions = HashMap::new();
    let mut namespace = HashSet::new();
    for (name, ty) in &function.parameters {
        if !namespace.insert(name.as_str()) {
            return Err(error(lines.line, format!("duplicate parameter %{name}")));
        }
        definitions.insert(
            name.as_str(),
            Definition {
                ty: ty.clone(),
                block: None,
                position: 0,
            },
        );
    }
    let mut labels = HashMap::new();
    for (index, block) in function.blocks.iter().enumerate() {
        if !namespace.insert(block.label.as_str()) {
            return Err(error(
                lines.blocks[index],
                format!("duplicate local identifier {}", block.label),
            ));
        }
        labels.insert(block.label.as_str(), index);
    }
    for (block_index, block) in function.blocks.iter().enumerate() {
        let mut non_phi = false;
        if block.instructions.is_empty() {
            return Err(error(
                lines.blocks[block_index],
                format!("basic block %{} has no terminator", block.label),
            ));
        }
        for (position, instruction) in block.instructions.iter().enumerate() {
            let line = lines.instructions[block_index][position];
            if position + 1 != block.instructions.len() && terminator(instruction) {
                return Err(error(line, "instruction follows a basic block terminator"));
            }
            if let Instruction::Phi { .. } = instruction {
                if non_phi {
                    return Err(error(
                        line,
                        "PHI instructions must precede all non-PHI instructions",
                    ));
                }
            } else {
                non_phi = true;
            }
            if matches!(instruction, Instruction::Alloca { .. }) && block_index != 0 {
                return Err(error(
                    line,
                    "static alloca is supported only in the entry block",
                ));
            }
            if let Some((name, ty)) = result(instruction) {
                if !namespace.insert(name) {
                    return Err(error(line, format!("duplicate SSA identifier %{name}")));
                }
                definitions.insert(
                    name,
                    Definition {
                        ty,
                        block: Some(block_index),
                        position,
                    },
                );
            }
            if let Instruction::Ret { ty, .. } = instruction {
                if *ty != function.return_type {
                    return Err(error(
                        line,
                        "return type does not match the function signature",
                    ));
                }
            }
        }
        if !terminator(block.instructions.last().unwrap()) {
            return Err(error(
                *lines.instructions[block_index].last().unwrap(),
                format!("basic block %{} has no terminator", block.label),
            ));
        }
    }
    let count = function.blocks.len();
    let mut predecessors = vec![Vec::new(); count];
    let mut successors = vec![Vec::new(); count];
    for (block_index, block) in function.blocks.iter().enumerate() {
        if let Instruction::Br {
            on_true, on_false, ..
        } = block.instructions.last().unwrap()
        {
            for target in std::iter::once(on_true).chain(on_false.iter()) {
                let target_index = *labels.get(target.as_str()).ok_or_else(|| {
                    error(
                        *lines.instructions[block_index].last().unwrap(),
                        format!("branch to unknown block %{target}"),
                    )
                })?;
                successors[block_index].push(target_index);
                predecessors[target_index].push(block_index);
            }
        }
    }
    if !predecessors[0].is_empty() {
        return Err(error(
            lines.blocks[0],
            "the entry block cannot have predecessors",
        ));
    }
    let mut reachable = vec![false; count];
    reachable[0] = true;
    let mut stack = vec![0];
    while let Some(block) = stack.pop() {
        for &successor in &successors[block] {
            if !reachable[successor] {
                reachable[successor] = true;
                stack.push(successor);
            }
        }
    }
    let all_reachable: HashSet<_> = (0..count).filter(|&index| reachable[index]).collect();
    let mut dominators: Vec<HashSet<usize>> = (0..count)
        .map(|index| {
            if index == 0 || !reachable[index] {
                HashSet::from([index])
            } else {
                all_reachable.clone()
            }
        })
        .collect();
    loop {
        let mut changed = false;
        for block in 1..count {
            if !reachable[block] {
                continue;
            }
            let mut incoming = predecessors[block]
                .iter()
                .copied()
                .filter(|&index| reachable[index]);
            let first = incoming.next().unwrap();
            let mut next = dominators[first].clone();
            for predecessor in incoming {
                next.retain(|index| dominators[predecessor].contains(index));
            }
            next.insert(block);
            if next != dominators[block] {
                dominators[block] = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let validation = Validation {
        definitions,
        globals: module
            .globals
            .iter()
            .map(|global| global.name.as_str())
            .collect(),
        dominators,
        reachable,
    };
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (position, instruction) in block.instructions.iter().enumerate() {
            let site = UseSite {
                block: block_index,
                position,
                line: lines.instructions[block_index][position],
            };
            let check = |operand: &Operand, ty: &Type| validation.operand(operand, ty, &site);
            match instruction {
                Instruction::Binary { ty, lhs, rhs, .. }
                | Instruction::ICmp { ty, lhs, rhs, .. } => {
                    check(lhs, ty)?;
                    check(rhs, ty)?;
                }
                Instruction::Select {
                    condition,
                    ty,
                    on_true,
                    on_false,
                    ..
                } => {
                    check(condition, &Type::Int(1))?;
                    check(on_true, ty)?;
                    check(on_false, ty)?;
                }
                Instruction::Cast { from, value, .. } => check(value, from)?,
                Instruction::Phi { ty, incoming, .. } => {
                    let mut expected = HashMap::<usize, usize>::new();
                    for &predecessor in &predecessors[block_index] {
                        *expected.entry(predecessor).or_default() += 1;
                    }
                    let mut actual = HashMap::<usize, (&Operand, usize)>::new();
                    for (value, predecessor) in incoming {
                        let &predecessor_index =
                            labels.get(predecessor.as_str()).ok_or_else(|| {
                                error(
                                    site.line,
                                    format!("PHI references unknown block %{predecessor}"),
                                )
                            })?;
                        if !expected.contains_key(&predecessor_index) {
                            return Err(error(
                                site.line,
                                format!("PHI incoming block %{predecessor} is not a predecessor"),
                            ));
                        }
                        if let Some((previous, occurrences)) = actual.get_mut(&predecessor_index) {
                            if *previous != value {
                                return Err(error(
                                    site.line,
                                    "duplicate PHI edges must carry the same value",
                                ));
                            }
                            *occurrences += 1;
                        } else {
                            actual.insert(predecessor_index, (value, 1));
                        }
                        let edge_site = UseSite {
                            block: predecessor_index,
                            position: function.blocks[predecessor_index].instructions.len(),
                            line: site.line,
                        };
                        validation.operand(value, ty, &edge_site)?;
                    }
                    if actual.len() != expected.len()
                        || expected.iter().any(|(predecessor, count)| {
                            actual.get(predecessor).map(|(_, n)| n) != Some(count)
                        })
                    {
                        return Err(error(
                            site.line,
                            "PHI incoming edges do not match the CFG predecessor edges",
                        ));
                    }
                }
                Instruction::Alloca { .. } => {}
                Instruction::Load { pointer, .. } => check(pointer, &Type::Ptr)?,
                Instruction::Store {
                    ty, value, pointer, ..
                } => {
                    check(value, ty)?;
                    check(pointer, &Type::Ptr)?;
                }
                Instruction::Gep { base, indices, .. } => {
                    check(base, &Type::Ptr)?;
                    for (ty, index) in indices {
                        check(index, ty)?;
                    }
                }
                Instruction::Call {
                    function: callee,
                    args,
                    return_type,
                    ..
                } => {
                    if module.globals.iter().any(|global| global.name == *callee) {
                        return Err(error(
                            site.line,
                            format!("@{callee} is a data global, not a function"),
                        ));
                    }
                    if let Some(signature) = signatures.get(callee) {
                        if signature.variadic {
                            return Err(error(site.line, "variadic calls are unsupported"));
                        }
                        if signature.return_type != *return_type
                            || args.len() != signature.parameters.len()
                            || args
                                .iter()
                                .zip(&signature.parameters)
                                .any(|((ty, _), expected)| ty != expected)
                        {
                            return Err(error(
                                site.line,
                                format!("call signature does not match @{callee}"),
                            ));
                        }
                    }
                    for (ty, value) in args {
                        check(value, ty)?;
                    }
                }
                Instruction::Br { condition, .. } => {
                    if let Some(condition) = condition {
                        check(condition, &Type::Int(1))?;
                    }
                }
                Instruction::Ret {
                    ty: Some(ty),
                    value: Some(value),
                } => check(value, ty)?,
                Instruction::Ret { .. } | Instruction::Unreachable => {}
            }
        }
    }
    Ok(())
}
