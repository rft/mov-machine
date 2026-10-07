@values = global [4 x i8] [i8 3, i8 7, i8 11, i8 21], align 1

define i8 @read_byte(ptr %base, i32 %index) {
entry:
  %address = getelementptr i8, ptr %base, i32 %index
  %value = load i8, ptr %address, align 1
  ret i8 %value
}

define i32 @main() {
entry:
  %base = getelementptr [4 x i8], ptr @values, i32 0, i32 0
  %before = call i8 @read_byte(ptr %base, i32 2)
  %address = getelementptr i8, ptr %base, i32 2
  %new = add i8 %before, 31
  store i8 %new, ptr %address, align 1
  %after = load i8, ptr %address, align 1
  %result = zext i8 %after to i32
  ret i32 %result
}
