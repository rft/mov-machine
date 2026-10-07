"""Version-1 MOVM executable framing and compiled bit-vector return ABI."""
from dataclasses import dataclass
from pathlib import Path
import struct

MAGIC = b"MOVM\x01\0\0\0"
HEADER = struct.Struct("<8sIIII8s")


@dataclass(frozen=True)
class ExecutableImage:
    program: tuple[int, ...]
    minimum_ram_words: int = 0
    result_address: int = 0
    result_bits: int = 0

    def decode_result(self, ram, halt_code):
        if not self.result_bits:
            return halt_code
        if halt_code != self.result_address:
            raise ValueError("compiled program halted with an invalid result address")
        value = 0
        for bit in range(self.result_bits):
            word = ram[self.result_address // 4 + bit]
            if word not in (0, 4):
                raise ValueError("compiled program returned a non-Boolean bit")
            value |= (word // 4) << bit
        return value

    def to_bytes(self):
        _validate(self)
        return HEADER.pack(MAGIC, len(self.program), self.minimum_ram_words,
                           self.result_address, self.result_bits, bytes(8)) + b"".join(
                               word.to_bytes(8, "little") for word in self.program)


def _validate(image):
    if not 0 <= image.minimum_ram_words <= 0xFFFFFFF8 // 4:
        raise ValueError("image exceeds the machine address space")
    if not 0 <= image.result_bits <= 32:
        raise ValueError("invalid encoded result width")
    if image.result_bits:
        if image.result_address & 3 or image.result_address + image.result_bits * 4 > image.minimum_ram_words * 4:
            raise ValueError("encoded result lies outside declared RAM")
    elif image.result_address:
        raise ValueError("native executable must not declare an encoded result address")
    if any(not 0 <= word < (1 << 64) for word in image.program):
        raise ValueError("instructions must fit in 64 bits")


def read_image(path):
    data = Path(path).read_bytes()
    if len(data) < HEADER.size:
        raise ValueError("expected a version-1 MOVM executable")
    magic, count, minimum, address, bits, reserved = HEADER.unpack_from(data)
    if magic != MAGIC:
        raise ValueError("expected a version-1 MOVM executable")
    if reserved != bytes(8) or len(data) != HEADER.size + count * 8:
        raise ValueError("malformed executable header or instruction count")
    image = ExecutableImage(tuple(word[0] for word in struct.iter_unpack("<Q", data[HEADER.size:])),
                            minimum, address, bits)
    _validate(image)
    return image
