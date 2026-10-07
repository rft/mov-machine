// Conditional control flow via a RAM lookup and MOV-to-PC, not a branch opcode.
// Change selector r0 between 0 and 4 to select one of two instruction indices.
.text
.globl _start
_start:
    mov r1, 0
    mov r2, zero
    mov [r1], r2
    mov r2, one
    mov [r1 + 4], r2
    mov r0, 4
    mov r2, [r0]
    mov r3, 0xfffffffc
    mov [r3], r2
zero:
    mov r0, 10
    mov r3, 0xfffffff8
    mov [r3], r0
one:
    mov r0, 42
    mov r3, 0xfffffff8
    mov [r3], r0
