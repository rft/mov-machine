use mov_machine::{assemble, Image, Machine, DEFAULT_RAM_WORDS};
use std::{
    env, fs,
    io::{self, Write},
    process::ExitCode,
};

fn execute() -> Result<bool, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let usage = "usage: mov-machine assemble INPUT.s OUTPUT.mov | (run INPUT.mov | exec INPUT.s) --max-steps N [--ram-words N] [--trace]";
    let command = args.first().ok_or(usage)?;
    if command == "assemble" {
        if args.len() != 3 {
            return Err(usage.into());
        }
        let source = fs::read_to_string(&args[1]).map_err(|e| format!("{}: {e}", args[1]))?;
        let image = assemble(&source).map_err(|e| e.to_string())?;
        fs::write(&args[2], image.to_bytes()).map_err(|e| format!("{}: {e}", args[2]))?;
        return Ok(true);
    }
    if command != "run" && command != "exec" {
        return Err(usage.into());
    }
    let input = args.get(1).ok_or(usage)?;
    let mut max_steps = None;
    let mut ram_words = None;
    let mut trace = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--trace" if !trace => {
                trace = true;
                index += 1;
            }
            "--max-steps" if max_steps.is_none() => {
                max_steps = Some(
                    args.get(index + 1)
                        .ok_or("--max-steps requires an unsigned integer")?
                        .parse::<u64>()
                        .map_err(|_| "invalid --max-steps")?,
                );
                index += 2;
            }
            "--ram-words" if ram_words.is_none() => {
                ram_words = Some(
                    args.get(index + 1)
                        .ok_or("--ram-words requires an unsigned integer")?
                        .parse::<usize>()
                        .map_err(|_| "invalid --ram-words")?,
                );
                index += 2;
            }
            option => return Err(format!("unknown or repeated option '{option}'")),
        }
    }
    let max_steps = max_steps.ok_or("--max-steps is required")?;
    let image = if command == "exec" {
        assemble(&fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?)
            .map_err(|e| e.to_string())?
    } else {
        Image::from_bytes(&fs::read(input).map_err(|e| format!("{input}: {e}"))?)
            .map_err(|e| e.to_string())?
    };
    let mut machine =
        Machine::new(image, ram_words.unwrap_or(DEFAULT_RAM_WORDS)).map_err(|e| e.to_string())?;
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
    writeln!(output, "{}", state.to_json()).map_err(|e| e.to_string())?;
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
