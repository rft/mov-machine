//! Local MOV image format, strict assembly adapter, and architectural emulator.
use std::collections::HashMap;
use std::fmt;

pub const PC_ADDRESS: u32 = 0xffff_fffc;
pub const HALT_ADDRESS: u32 = 0xffff_fff8;
pub const DEFAULT_RAM_WORDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub header: u32,
    pub extension: u32,
}
impl Instruction {
    pub fn new(opcode: u32, dst: u32, src: u32, extension: u32) -> Self {
        Self {
            header: (opcode << 26) | (dst << 24) | (src << 22),
            extension,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    instructions: Vec<Instruction>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageError(pub String);
impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ImageError {}
impl Image {
    pub fn new(instructions: Vec<Instruction>) -> Self {
        Self { instructions }
    }
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ImageError> {
        if !bytes.len().is_multiple_of(8) {
            return Err(ImageError(
                "image length must be a multiple of 8 bytes".into(),
            ));
        }
        let instructions = bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| Instruction {
                header: u32::from_le_bytes(b[0..4].try_into().unwrap()),
                extension: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            })
            .collect();
        Ok(Self { instructions })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.instructions.len() * 8);
        for i in &self.instructions {
            bytes.extend_from_slice(&i.header.to_le_bytes());
            bytes.extend_from_slice(&i.extension.to_le_bytes());
        }
        bytes
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub pc: u32,
    pub registers: [u32; 4],
    pub halted: bool,
    pub fault: bool,
    pub halt_code: u32,
    pub steps: u64,
}
impl State {
    pub fn to_json(&self) -> String {
        format!("{{\"pc\":{},\"registers\":[{},{},{},{}],\"halted\":{},\"fault\":{},\"halt_code\":{},\"steps\":{}}}", self.pc, self.registers[0], self.registers[1], self.registers[2], self.registers[3], self.halted, self.fault, self.halt_code, self.steps)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryWrite {
    pub address: u32,
    pub value: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepOutcome {
    pub executed: bool,
    pub retired: bool,
    pub state: State,
    pub memory_write: Option<MemoryWrite>,
}
impl StepOutcome {
    pub fn to_json(&self) -> String {
        let mut json = self.state.to_json();
        json.pop();
        json.push_str(",\"memory_write\":");
        match self.memory_write {
            Some(w) => json.push_str(&format!(
                "{{\"address\":{},\"value\":{}}}",
                w.address, w.value
            )),
            None => json.push_str("null"),
        }
        json.push('}');
        json
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Halted,
    Fault,
    BudgetExhausted,
}
#[derive(Debug)]
pub struct MachineError(pub String);
impl fmt::Display for MachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MachineError {}
pub struct Machine {
    image: Image,
    ram: Vec<u32>,
    state: State,
}
impl Machine {
    pub fn new(image: Image, ram_words: usize) -> Result<Self, MachineError> {
        // RAM must not overlap the two local MMIO words.
        if ram_words as u128 > HALT_ADDRESS as u128 / 4 {
            return Err(MachineError("RAM overlaps MMIO address space".into()));
        }
        let mut ram = Vec::new();
        ram.try_reserve_exact(ram_words)
            .map_err(|e| MachineError(format!("cannot allocate RAM: {e}")))?;
        ram.resize(ram_words, 0);
        Ok(Self {
            image,
            ram,
            state: State {
                pc: 0,
                registers: [0; 4],
                halted: false,
                fault: false,
                halt_code: 0,
                steps: 0,
            },
        })
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn ram(&self) -> &[u32] {
        &self.ram
    }
    pub fn ram_mut(&mut self) -> &mut [u32] {
        &mut self.ram
    }
    pub fn reset(&mut self) {
        self.state = State {
            pc: 0,
            registers: [0; 4],
            halted: false,
            fault: false,
            halt_code: 0,
            steps: 0,
        };
    }
    fn fail(&mut self) -> StepOutcome {
        self.state.halted = true;
        self.state.fault = true;
        StepOutcome {
            executed: true,
            retired: false,
            state: self.state,
            memory_write: None,
        }
    }
    pub fn step(&mut self) -> StepOutcome {
        if self.state.halted {
            return StepOutcome {
                executed: false,
                retired: false,
                state: self.state,
                memory_write: None,
            };
        }
        self.state.steps = self.state.steps.saturating_add(1);
        let Some(&instruction) = self.image.instructions.get(self.state.pc as usize) else {
            return self.fail();
        };
        let opcode = instruction.header >> 26;
        let dst = ((instruction.header >> 24) & 3) as usize;
        let src = ((instruction.header >> 22) & 3) as usize;
        if opcode > 3
            || instruction.header & 0x3f_ffff != 0
            || (opcode == 0 && instruction.extension != 0)
            || (opcode == 1 && src != 0)
        {
            return self.fail();
        }
        let mut next_pc = self.state.pc.wrapping_add(1);
        let mut memory_write = None;
        match opcode {
            0 => self.state.registers[dst] = self.state.registers[src],
            1 => self.state.registers[dst] = instruction.extension,
            2 | 3 => {
                let base = if opcode == 2 { src } else { dst };
                let address = self.state.registers[base].wrapping_add(instruction.extension);
                if address & 3 != 0 {
                    return self.fail();
                }
                if opcode == 2 {
                    let value = if address == PC_ADDRESS {
                        self.state.pc
                    } else if address == HALT_ADDRESS {
                        return self.fail();
                    } else {
                        let Some(&value) = self.ram.get((address / 4) as usize) else {
                            return self.fail();
                        };
                        value
                    };
                    self.state.registers[dst] = value;
                } else {
                    let value = self.state.registers[src];
                    if address == PC_ADDRESS {
                        next_pc = value;
                    } else if address == HALT_ADDRESS {
                        self.state.halted = true;
                        self.state.halt_code = value;
                    } else {
                        let Some(word) = self.ram.get_mut((address / 4) as usize) else {
                            return self.fail();
                        };
                        *word = value;
                    }
                    memory_write = Some(MemoryWrite { address, value });
                }
            }
            _ => unreachable!(),
        }
        self.state.pc = next_pc;
        StepOutcome {
            executed: true,
            retired: true,
            state: self.state,
            memory_write,
        }
    }
    pub fn run(&mut self, max_steps: u64) -> RunOutcome {
        for _ in 0..max_steps {
            if self.state.halted {
                break;
            }
            self.step();
        }
        if self.state.fault {
            RunOutcome::Fault
        } else if self.state.halted {
            RunOutcome::Halted
        } else {
            RunOutcome::BudgetExhausted
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyError {
    pub line: usize,
    pub message: String,
}
impl fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}
impl std::error::Error for AssemblyError {}
fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '.')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$')
}
fn register(s: &str) -> Option<u32> {
    match s.trim() {
        "r0" => Some(0),
        "r1" => Some(1),
        "r2" => Some(2),
        "r3" => Some(3),
        _ => None,
    }
}
fn number(s: &str) -> Result<u32, String> {
    let s = s.trim();
    let (negative, digits) = if let Some(v) = s.strip_prefix('-') {
        (true, v)
    } else {
        (false, s.strip_prefix('+').unwrap_or(s))
    };
    let value = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else {
        digits.parse::<u64>()
    }
    .map_err(|_| format!("invalid immediate '{s}'"))?;
    if negative {
        if value > 0x8000_0000 {
            return Err(format!("immediate out of 32-bit range: {s}"));
        }
        Ok((0u32).wrapping_sub(value as u32))
    } else {
        u32::try_from(value).map_err(|_| format!("immediate out of 32-bit range: {s}"))
    }
}
fn immediate(s: &str, labels: &HashMap<String, u32>) -> Result<u32, String> {
    let s = s.trim().strip_prefix('#').unwrap_or(s.trim()).trim();
    if let Some(&value) = labels.get(s) {
        Ok(value)
    } else {
        number(s)
    }
}
fn memory(s: &str) -> Result<(u32, u32), String> {
    let inside = s
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or_else(|| "expected [register +/- offset]".to_string())?
        .trim();
    if inside.len() < 2 || !inside.is_ascii() {
        return Err("invalid memory operand".into());
    }
    let base = register(&inside[..2]).ok_or_else(|| "invalid memory base register".to_string())?;
    let remainder = inside[2..].trim();
    if remainder.is_empty() {
        return Ok((base, 0));
    }
    let (sign, magnitude) = if let Some(v) = remainder.strip_prefix('+') {
        ('+', v.trim())
    } else if let Some(v) = remainder.strip_prefix('-') {
        ('-', v.trim())
    } else {
        return Err("memory offset requires + or -".into());
    };
    if magnitude.starts_with('+') || magnitude.starts_with('-') {
        return Err("invalid memory offset".into());
    }
    let offset = number(&format!("{sign}{magnitude}"))?;
    Ok((base, offset))
}
fn parse_instruction(text: &str, labels: &HashMap<String, u32>) -> Result<Instruction, String> {
    let split = text
        .find(char::is_whitespace)
        .ok_or_else(|| "instruction requires two operands".to_string())?;
    let mnemonic = &text[..split];
    let expected = match mnemonic {
        "mov" => None,
        "mov.rr" => Some(0),
        "mov.ri" => Some(1),
        "mov.rm" => Some(2),
        "mov.mr" => Some(3),
        _ => return Err(format!("unsupported instruction '{mnemonic}'")),
    };
    let mut operands = text[split..].trim().split(',');
    let dst = operands.next().unwrap_or("").trim();
    let src = operands
        .next()
        .ok_or_else(|| "instruction requires two operands".to_string())?
        .trim();
    if operands.next().is_some() || dst.is_empty() || src.is_empty() {
        return Err("instruction requires exactly two nonempty operands".into());
    }
    let instruction = if let Some(d) = register(dst) {
        if let Some(s) = register(src) {
            Instruction::new(0, d, s, 0)
        } else if src.starts_with('[') {
            let (s, offset) = memory(src)?;
            Instruction::new(2, d, s, offset)
        } else {
            Instruction::new(1, d, 0, immediate(src, labels)?)
        }
    } else if dst.starts_with('[') {
        let (d, offset) = memory(dst)?;
        let s = register(src).ok_or_else(|| "store source must be a register".to_string())?;
        Instruction::new(3, d, s, offset)
    } else {
        return Err("destination must be a register or memory operand".into());
    };
    if expected.is_some_and(|op| op != instruction.header >> 26) {
        return Err("suffix does not match operand form".into());
    }
    Ok(instruction)
}
/// Assemble the four destination-first MOV forms. Labels denote instruction indices.
pub fn assemble(source: &str) -> Result<Image, AssemblyError> {
    let mut labels = HashMap::new();
    let mut lines = Vec::new();
    for (index, original) in source.lines().enumerate() {
        let line = index + 1;
        let fail = |message: String| AssemblyError { line, message };
        let comment = original
            .find(';')
            .into_iter()
            .chain(original.find("//"))
            .min()
            .unwrap_or(original.len());
        let mut text = original[..comment].trim();
        while let Some(colon) = text.find(':') {
            let label = text[..colon].trim();
            if !identifier(label) {
                return Err(fail("invalid label".into()));
            }
            let pc =
                u32::try_from(lines.len()).map_err(|_| fail("too many instructions".into()))?;
            if labels.insert(label.to_string(), pc).is_some() {
                return Err(fail(format!("duplicate label '{label}'")));
            }
            text = text[colon + 1..].trim();
        }
        if text.is_empty() {
            continue;
        }
        if text.starts_with('.') {
            let split = text.find(char::is_whitespace).unwrap_or(text.len());
            let directive = &text[..split];
            let args = text[split..].trim();
            match directive {
                ".text" if args.is_empty() => (),
                ".globl" | ".global"
                    if !args.is_empty() && args.split(',').all(|s| identifier(s.trim())) => {}
                ".type" | ".size"
                    if args.split_once(',').is_some_and(|(name, value)| {
                        identifier(name.trim()) && !value.trim().is_empty()
                    }) => {}
                ".p2align"
                    if !args.is_empty() && args.split(',').all(|s| number(s.trim()).is_ok()) => {}
                _ => {
                    return Err(fail(format!(
                        "unsupported or malformed directive '{directive}'"
                    )))
                }
            }
            continue;
        }
        lines.push((line, text));
    }
    let instructions = lines
        .into_iter()
        .map(|(line, text)| {
            parse_instruction(text, &labels).map_err(|message| AssemblyError { line, message })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Image::new(instructions))
}
