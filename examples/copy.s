// Destination-first syntax exercising the local machine's four MOV forms.
.text
.globl _start
_start:
    mov.ri r0, #42
    mov r1, 0
    mov.mr [r1 + 4], r0
    mov.rm r2, [r1 + 4]
    mov.rr r0, r2
    mov r3, 0xfffffffc
    mov r1, done
    mov [r3], r1
    mov r0, 0xbad
 done:
    mov r3, 0xfffffff8
    mov [r3], r0
