"""Behavioral integration: Rust CLI images versus cycle-accurate Amaranth RTL."""
import argparse
import json
from pathlib import Path
import random
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "hw"))
from amaranth.sim import Simulator
from mov_machine.core import MovCore
from mov_machine.soc import MovSoC


def command(*args, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def snapshot(ctx, core, steps):
    return {"pc": ctx.get(core.pc), "registers": [ctx.get(r) for r in core.regs],
            "halted": bool(ctx.get(core.halted)), "fault": bool(ctx.get(core.fault)),
            "halt_code": ctx.get(core.halt_code), "steps": steps}


def simulate(program, ram_words, stalls=False):
    """Compare at architectural boundaries, not at implementation cycle counts."""
    dut = MovCore() if stalls else MovSoC(program, ram_words)
    core = dut if stalls else dut.core
    sim = Simulator(dut)
    sim.add_clock(1e-6)
    trace = []
    memory = [0] * ram_words
    rng = random.Random(714)

    async def bench(ctx):
        ctx.set(dut.reset, 1)
        await ctx.tick()
        ctx.set(dut.reset, 0)
        for _ in range(20000):
            if stalls:
                pc = ctx.get(core.pc)
                address = ctx.get(core.dmem_addr)
                ctx.set(core.imem_ready, rng.randrange(4) == 0)
                ctx.set(core.imem_error, pc >= len(program))
                ctx.set(core.imem_data, program[pc] if pc < len(program) else 0)
                ctx.set(core.dmem_ready, rng.randrange(4) == 0)
                ctx.set(core.dmem_error, address // 4 >= ram_words)
                ctx.set(core.dmem_rdata, memory[address // 4] if address // 4 < ram_words else 0)
            stopped = ctx.get(core.halted)
            retired = ctx.get(core.retire)
            write = None
            if retired and ((ctx.get(core.instruction) >> 26) & 63) == 3:
                write = {"address": ctx.get(core.dmem_addr), "value": ctx.get(core.dmem_wdata)}
            if stalls and ctx.get(core.dmem_valid) and ctx.get(core.dmem_ready) and not ctx.get(core.dmem_error) and ctx.get(core.dmem_write):
                memory[ctx.get(core.dmem_addr) // 4] = ctx.get(core.dmem_wdata)
            before = snapshot(ctx, core, len(trace))
            waiting = (ctx.get(core.imem_valid) and not ctx.get(core.imem_ready)) or (ctx.get(core.dmem_valid) and not ctx.get(core.dmem_ready))
            request = (ctx.get(core.imem_addr), ctx.get(core.dmem_addr), ctx.get(core.dmem_wdata), ctx.get(core.dmem_write))
            await ctx.tick()
            if waiting:
                assert snapshot(ctx, core, len(trace)) == before, "stalled architectural state changed"
                assert request == (ctx.get(core.imem_addr), ctx.get(core.dmem_addr), ctx.get(core.dmem_wdata), ctx.get(core.dmem_write)), "stalled request changed"
            if retired or (not stopped and ctx.get(core.halted)):
                state = snapshot(ctx, core, len(trace) + 1)
                state["memory_write"] = write
                trace.append(state)
            if ctx.get(core.halted):
                # A halted machine must not restart or emit requests.
                final = snapshot(ctx, core, len(trace))
                for _ in range(3):
                    await ctx.tick()
                    assert not ctx.get(core.imem_valid) and not ctx.get(core.dmem_valid)
                    assert snapshot(ctx, core, len(trace)) == final
                return
        raise AssertionError("simulation cycle budget exhausted")

    sim.add_testbench(bench)
    sim.run()
    return trace


def reset_smoke():
    # Reset during an outstanding store must cancel it; earlier RAM survives reset.
    def ins(op, dst, src=0, ext=0):
        return (op << 26) | (dst << 24) | (src << 22) | (ext << 32)
    program = [ins(1, 0, ext=42), ins(1, 1, ext=4), ins(3, 1, 0),
               ins(1, 0, ext=99), ins(3, 1, 0), ins(1, 3, ext=0xfffffff8), ins(3, 3, 0)]
    soc = MovSoC(program, 8)
    sim = Simulator(soc)
    sim.add_clock(1e-6)

    async def bench(ctx):
        for _ in range(100):
            if ctx.get(soc.core.pc) == 4 and ctx.get(soc.core.dmem_valid) and ctx.get(soc.core.dmem_ready):
                ctx.set(soc.reset, 1)
                await ctx.tick()
                assert ctx.get(soc.core.pc) == 0 and not ctx.get(soc.core.halted)
                assert all(ctx.get(r) == 0 for r in soc.core.regs)
                assert ctx.get(soc.ram.data[1]) == 42, "reset committed cancelled store or cleared RAM"
                ctx.set(soc.reset, 0)
                for _ in range(100):
                    await ctx.tick()
                    if ctx.get(soc.core.halted):
                        assert ctx.get(soc.core.halt_code) == 99 and not ctx.get(soc.core.fault)
                        assert ctx.get(soc.ram.data[1]) == 99
                        return
                raise AssertionError("reset recovery did not complete")
            await ctx.tick()
        raise AssertionError("pending store was not reached")

    sim.add_testbench(bench)
    sim.run()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--random-cases", type=int, default=32)
    args = parser.parse_args()
    if args.random_cases < 0:
        parser.error("--random-cases must be nonnegative")
    command("cargo", "test", "--locked")
    command("cargo", "build", "--locked")
    executable = ROOT / "target/debug/mov-machine"
    rng = random.Random(20261007)
    cases = [(p.stem, p.read_text()) for p in sorted((ROOT / "examples").glob("*.s"))]
    cases += [
        ("alias-load", "mov r0, 4\nmov r1, -1\nmov [r0], r1\nmov r0, [r0]\nmov r3, 0xfffffff8\nmov [r3], r0"),
        ("wrapping-address", "mov r0, -4\nmov r1, 42\nmov [r0 + 8], r1\nmov r2, [r0 + 8]\nmov r3, 0xfffffff8\nmov [r3], r2"),
        ("pc-read", "mov r0, 0xfffffffc\nmov r1, [r0]\nmov r0, 0xfffffff8\nmov [r0], r1"),
        ("misaligned", "mov r0, 1\nmov r1, [r0]"),
        ("ram-limit", "mov r0, 1024\nmov r1, 42\nmov [r0], r1"),
        ("halt-read-fault", "mov r0, 0xfffffff8\nmov r1, [r0]"),
        ("fetch-fault", "mov r0, 42"),
        ("jump-outside", "mov r0, 0xfffffffc\nmov r1, -1\nmov [r0], r1"),
    ]
    for number in range(args.random_cases):
        lines = ["mov r3, 0"]
        for _ in range(24):
            dst, src = rng.randrange(3), rng.randrange(3)
            form = rng.randrange(4)
            offset = rng.randrange(16) * 4
            if form == 0:
                lines.append(f"mov r{dst}, r{src}")
            elif form == 1:
                lines.append(f"mov r{dst}, 0x{rng.getrandbits(32):x}")
            elif form == 2:
                lines.append(f"mov r{dst}, [r3 + {offset}]")
            else:
                lines.append(f"mov [r3 + {offset}], r{src}")
        lines += ["mov r3, 0xfffffff8", "mov [r3], r0"]
        cases.append((f"random-{number}", "\n".join(lines)))
    with tempfile.TemporaryDirectory(prefix="mov-check-") as temp:
        directory = Path(temp)
        images = []
        for name, source in cases:
            assembly, image = directory / f"{name}.s", directory / f"{name}.mov"
            assembly.write_text(source)
            command(str(executable), "assemble", str(assembly), str(image))
            images.append((name, image))
        for name, raw in [("bad-opcode", 4 << 26), ("reserved", 1),
                          ("rr-extension", 1 << 32), ("ri-src", (1 << 26) | (1 << 22))]:
            image = directory / f"{name}.mov"
            image.write_bytes(raw.to_bytes(8, "little"))
            images.append((name, image))
        for name, image in images:
            run = subprocess.run([str(executable), "run", str(image), "--max-steps", "1000", "--trace"], cwd=ROOT, capture_output=True, text=True)
            assert run.returncode in (0, 1), f"{name}: CLI failed: {run.stderr}"
            rows = [json.loads(row) for row in run.stdout.splitlines()]
            expected, final = rows[:-1], rows[-1]
            assert final["halted"], f"{name}: did not terminate"
            program = [int.from_bytes(image.read_bytes()[i:i + 8], "little") for i in range(0, image.stat().st_size, 8)]
            for stalls in (False, True):
                actual = simulate(program, 256, stalls)
                assert actual == expected, f"{name}, stalls={stalls}:\nRTL {actual}\nRust {expected}"
            print(f"PASS {name}: {len(expected)} architectural steps (SoC and stalled core)")
    reset_smoke()
    print("PASS reset: cancelled store, preserved RAM, successful restart")
    print(f"PASS {len(images)} differential programs")


if __name__ == "__main__":
    main()
