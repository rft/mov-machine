#!/usr/bin/env python3
"""Generate actual-core RTL harnesses and run SBY/Z3, TLC and Alloy 6.

Default: all checks, sequentially. Use --generate-only to emit RTL/SBY files
without running tools. Paths are independent of the invoking working directory.
Required executables: sby, yosys, yosys-smtbmc, z3, tlc, alloy6 (Alloy 6 CLI).
Jar alternatives: --tlc-jar / TLA2TOOLS_JAR, --alloy-jar / ALLOY_JAR and java.
No solver status is manufactured: tool errors and counterexamples fail the run.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

FORMAL = Path(__file__).resolve().parent
ROOT = FORMAL.parent
CHECKS = {"FaultImpliesStop", "PreciseFault", "StopSticky", "RegisterFrame",
          "MemoryFrame", "PCTransition", "ResetPreservesRAM", "MoveSemantics"}
WITNESSES = {"CanReset", "CanStop", "CanStall", "CanFetch", "CanFetchFault",
             "CanRR", "CanRI", "CanLoad", "CanStore", "CanJump", "CanHalt",
             "CanBadInstruction", "CanMemoryFault"}


def executable(name):
    resolved = shutil.which(name)
    if not resolved:
        raise RuntimeError(f"required executable not found: {name}")
    return resolved


def launch(argv, cwd):
    print("+ " + shlex.join(str(a) for a in argv), flush=True)
    subprocess.run([str(a) for a in argv], cwd=cwd, check=True)


def java_tool(wrapper, jar, java):
    if jar:
        path = Path(jar).expanduser().resolve()
        if not path.is_file():
            raise RuntimeError(f"jar does not exist: {path}")
        return [executable(java), "-jar", str(path)]
    return [executable(wrapper)]


def generate(build, selected):
    # Import only for RTL targets, so finite models also run without Amaranth.
    sys.path.insert(0, str(ROOT / "hw"))
    from amaranth.back import rtlil
    from harness import CoreFormal, SoCFormal
    for name, factory in (("core", CoreFormal), ("soc", SoCFormal)):
        if name not in selected:
            continue
        top = name + "_formal"
        (build / (name + ".il")).write_text(
            rtlil.convert(factory(), name=top, ports=[]), encoding="utf-8")
        (build / (name + ".sby")).write_text(f"""[tasks]
prove
cover

[options]
prove: mode prove
prove: depth 6
cover: mode cover
cover: depth 64

[engines]
smtbmc z3

[script]
read_rtlil {name}.il
proc
memory_map
prep -top {top}

[files]
{name}.il
""", encoding="utf-8")
        print(f"Generated {build / (name + '.il')} and {build / (name + '.sby')}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=("all", "core", "soc", "tla", "alloy"), default="all")
    parser.add_argument("--generate-only", action="store_true")
    parser.add_argument("--build-dir", type=Path, default=ROOT / "build" / "formal")
    parser.add_argument("--sby", default="sby")
    parser.add_argument("--tlc", default="tlc")
    parser.add_argument("--alloy", default="alloy6")
    parser.add_argument("--tlc-jar", default=os.environ.get("TLA2TOOLS_JAR"))
    parser.add_argument("--alloy-jar", default=os.environ.get("ALLOY_JAR"))
    parser.add_argument("--java", default=os.environ.get("JAVA", "java"))
    parser.add_argument("--workers", type=int, default=1, help="TLC worker count (default: 1)")
    args = parser.parse_args()
    if args.workers < 1:
        parser.error("--workers must be positive")
    if args.generate_only and args.target in ("tla", "alloy"):
        parser.error("--generate-only applies to core/soc/all RTL generation")
    selected = {"core", "soc", "tla", "alloy"} if args.target == "all" else {args.target}
    build = args.build_dir.expanduser().resolve()
    build.mkdir(parents=True, exist_ok=True)
    if selected & {"core", "soc"}:
        generate(build, selected)
    if args.generate_only:
        return
    if selected & {"core", "soc"}:
        sby = executable(args.sby)
        for name in ("core", "soc"):
            if name in selected:
                # Separate invocations prevent proof and cover CPU contention.
                for task in ("prove", "cover"):
                    launch([sby, "-f", "-d", build / f"{name}_{task}",
                            build / (name + ".sby"), task], build)
    if "tla" in selected:
        launch(java_tool(args.tlc, args.tlc_jar, args.java) +
               ["-workers", str(args.workers), "-metadir", str(build / "tlc"),
                "-config", str(FORMAL / "MovMachine.cfg"), str(FORMAL / "MovMachine.tla")], build)
    if "alloy" in selected:
        out = build / "alloy"
        launch(java_tool(args.alloy, args.alloy_jar, args.java) +
               ["exec", "-f", "-c", "*", "-t", "json", "-o", str(out),
                str(FORMAL / "mov_machine.als")], build)
        # CLI may exit successfully with a SAT counterexample. Require the
        # receipt to include every command and check actual solver solutions.
        receipt = json.loads((out / "receipt.json").read_text(encoding="utf-8"))
        commands = receipt["commands"]
        if set(commands) != CHECKS | WITNESSES:
            raise RuntimeError("Alloy receipt is missing or adding model commands")
        for name, result in commands.items():
            if result.get("transformPath"):
                raise RuntimeError(f"Alloy transformed rather than solved {name}")
            if result.get("type") != ("check" if name in CHECKS else "run"):
                raise RuntimeError(f"unexpected Alloy command type for {name}")
            sat = bool(result.get("solution", []))
            if sat != (name in WITNESSES):
                raise RuntimeError(f"Alloy {'counterexample' if sat else 'unreachable witness'}: {name}")
        print(f"Validated Alloy solver receipt: {out / 'receipt.json'}", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"formal verification failed: {error}", file=sys.stderr)
        sys.exit(1)
