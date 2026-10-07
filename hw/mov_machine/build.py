"""Build a MOV image into an iCEBreaker bitstream, or export board-neutral RTL."""
import argparse
from pathlib import Path

from amaranth import Elaboratable, Module, Signal
from amaranth.back import rtlil, verilog
from amaranth.lib.cdc import FFSynchronizer
from amaranth_boards.icebreaker import ICEBreakerPlatform

from .image import read_image
from .soc import MovSoC


class ICEBreakerMachine(Elaboratable):
    def __init__(self, program, ram_words):
        self.soc = MovSoC(program, ram_words)

    def elaborate(self, platform):
        m = Module()
        m.submodules.soc = self.soc
        button = platform.request("button", 0)
        reset = Signal(init=1)
        m.submodules.button_sync = FFSynchronizer(button.i, reset, init=1)
        m.d.comb += [
            self.soc.reset.eq(reset),
            platform.request("led_g", 0).o.eq(self.soc.core.halted & ~self.soc.core.fault),
            platform.request("led_r", 0).o.eq(self.soc.core.fault),
        ]
        return m


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("--build-dir", type=Path, default=Path("build/icebreaker"))
    parser.add_argument("--ram-words", type=int, help="RAM capacity; defaults to max(256, image requirement)")
    parser.add_argument("--program", action="store_true", help="flash a connected board using iceprog")
    parser.add_argument("--emit-rtl", type=Path, help="export .il or .v instead of building a bitstream")
    args = parser.parse_args()
    try:
        image = read_image(args.image)
        program = image.program
        ram_words = args.ram_words if args.ram_words is not None else max(256, image.minimum_ram_words)
        if ram_words < image.minimum_ram_words:
            parser.error(f"image requires at least {image.minimum_ram_words} RAM words")
        if args.emit_rtl:
            if args.program:
                parser.error("--program cannot be combined with --emit-rtl")
            soc = MovSoC(program, ram_words)
            if args.emit_rtl.suffix not in (".il", ".v"):
                parser.error("--emit-rtl path must end in .il or .v")
            converter = rtlil if args.emit_rtl.suffix == ".il" else verilog
            text = converter.convert(soc, name="mov_soc", ports=[soc.reset, soc.core.pc,
                                     *soc.core.regs, soc.core.halted,
                                     soc.core.fault, soc.core.halt_code])
            args.emit_rtl.parent.mkdir(parents=True, exist_ok=True)
            args.emit_rtl.write_text(text)
            print(args.emit_rtl)
        else:
            top = ICEBreakerMachine(program, ram_words)
            platform = ICEBreakerPlatform()
            platform.build(top, name="mov_machine", build_dir=str(args.build_dir),
                           do_program=args.program)
            print(args.build_dir / "mov_machine.bin")
    except (ValueError, OSError) as error:
        parser.exit(2, f"{error}\n")


if __name__ == "__main__":
    main()
