use mov_machine::{
    assemble,
    compiler::{compile_ir, CompileOptions},
    Image, Machine, DEFAULT_RAM_WORDS,
};
use std::{
    env, fs,
    io::{self, Write},
    process::ExitCode,
};

fn argument<'a>(args: &'a [String], index: &mut usize, option: &str) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

fn word(value: &str) -> Result<u32, String> {
    if let Some(hex) = value.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).map_err(|_| format!("invalid 32-bit argument: {value}"))
    } else {
        let number = value
            .parse::<i64>()
            .map_err(|_| format!("invalid 32-bit argument: {value}"))?;
        if !(-2147483648..=4294967295).contains(&number) {
            return Err(format!("argument outside 32-bit range: {value}"));
        }
        Ok(number as u32)
    }
}

fn execute() -> Result<bool, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let usage = "usage: mov-machine assemble INPUT.s OUTPUT.mov | compile INPUT.ll OUTPUT.mov [--entry NAME] [--arg VALUE] [--ram-words N] [--memory-bytes N] | (run INPUT.mov | exec INPUT.s | exec-ir INPUT.ll) --max-steps N [--ram-words N] [--trace]";
    let command = args.first().ok_or(usage)?.as_str();
    let input = args.get(1).ok_or(usage)?;
    if command == "assemble" {
        if args.len() != 3 {
            return Err(usage.into());
        }
        let source = fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?;
        let image = assemble(&source).map_err(|e| e.to_string())?;
        fs::write(&args[2], image.to_bytes()).map_err(|e| format!("{}: {e}", args[2]))?;
        return Ok(true);
    }
    if !matches!(command, "compile" | "run" | "exec" | "exec-ir") {
        return Err(usage.into());
    }
    let compiling = matches!(command, "compile" | "exec-ir");
    let output_path = if command == "compile" {
        Some(args.get(2).ok_or(usage)?)
    } else {
        None
    };
    let mut options = CompileOptions::default();
    let mut max_steps = None;
    let mut ram_words = None;
    let mut memory_bytes = None;
    let mut entry = None;
    let mut trace = false;
    let mut index = if output_path.is_some() { 3 } else { 2 };
    while index < args.len() {
        match args[index].as_str() {
            "--entry" if compiling && entry.is_none() => {
                entry = Some(argument(&args, &mut index, "--entry")?.to_owned());
            }
            "--arg" if compiling => options
                .arguments
                .push(word(argument(&args, &mut index, "--arg")?)?),
            "--memory-bytes" if compiling && memory_bytes.is_none() => {
                memory_bytes = Some(
                    argument(&args, &mut index, "--memory-bytes")?
                        .parse::<usize>()
                        .map_err(|_| "invalid --memory-bytes")?,
                );
            }
            "--trace" if output_path.is_none() && !trace => trace = true,
            "--max-steps" if output_path.is_none() && max_steps.is_none() => {
                max_steps = Some(
                    argument(&args, &mut index, "--max-steps")?
                        .parse::<u64>()
                        .map_err(|_| "invalid --max-steps")?,
                );
            }
            "--ram-words" if ram_words.is_none() => {
                ram_words = Some(
                    argument(&args, &mut index, "--ram-words")?
                        .parse::<usize>()
                        .map_err(|_| "invalid --ram-words")?,
                );
            }
            option => return Err(format!("unknown or repeated option '{option}'")),
        }
        index += 1;
    }
    if output_path.is_none() && max_steps.is_none() {
        return Err("--max-steps is required".into());
    }
    if let Some(entry) = entry {
        options.entry = entry;
    }
    if let Some(capacity) = ram_words {
        options.ram_words = capacity;
    }
    if let Some(capacity) = memory_bytes {
        options.memory_bytes = capacity;
    }
    let image = if compiling {
        let source = fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?;
        let compiled = compile_ir(&source, &options).map_err(|e| e.to_string())?;
        eprintln!(
            "Compiled {} MOV instructions; requires {} RAM words; {} logical memory bytes",
            compiled.image.instructions().len(),
            compiled.required_ram_words,
            compiled.logical_memory_bytes
        );
        compiled.image
    } else if command == "exec" {
        assemble(&fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?)
            .map_err(|e| e.to_string())?
    } else {
        Image::from_bytes(&fs::read(input).map_err(|e| format!("{input}: {e}"))?)
            .map_err(|e| e.to_string())?
    };
    if let Some(path) = output_path {
        fs::write(path, image.to_bytes()).map_err(|e| format!("{path}: {e}"))?;
        return Ok(true);
    }
    let max_steps = max_steps.ok_or("--max-steps is required")?;
    let capacity = ram_words.unwrap_or(DEFAULT_RAM_WORDS.max(image.minimum_ram_words()));
    let encoded_result = image.result_layout().is_some();
    let mut machine = Machine::new(image, capacity).map_err(|e| e.to_string())?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for _ in 0..max_steps {
        if machine.state().halted {
            break;
        }
        let step = machine.step();
        if trace {
            writeln!(output, "{}", step.to_json()).map_err(|e| e.to_string())?;
        }
    }
    let state = machine.state();
    let mut json = state.to_json();
    if encoded_result {
        let result = machine.result().map_err(|e| e.to_string())?;
        json.pop();
        json.push_str(",\"result\":");
        if let Some(result) = result {
            json.push_str(&result.to_string());
        } else {
            json.push_str("null");
        }
        json.push('}');
    }
    writeln!(output, "{json}").map_err(|e| e.to_string())?;
    output.flush().map_err(|e| e.to_string())?;
    if state.fault {
        eprintln!("machine fault");
    } else if !state.halted {
        eprintln!("step budget exhausted");
    }
    Ok(state.halted && !state.fault)
}

fn main() -> ExitCode {
    match execute() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
