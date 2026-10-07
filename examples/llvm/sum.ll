; Runtime arithmetic through MOV-only Boolean gates, not a constant add opcode.
define i8 @sum(i8 %a, i8 %b) {
entry:
  %value = add i8 %a, %b
  ret i8 %value
}

define i32 @main() {
entry:
  %small = call i8 @sum(i8 17, i8 25)
  %result = zext i8 %small to i32
  ret i32 %result
}
