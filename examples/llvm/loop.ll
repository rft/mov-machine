; Sum 1 through 10, exercising loop-carried SSA and branch selection.
define i32 @main() {
entry:
  br label %loop
loop:
  %index = phi i8 [ 1, %entry ], [ %next, %body ]
  %sum = phi i8 [ 0, %entry ], [ %added, %body ]
  %continue = icmp ule i8 %index, 10
  br i1 %continue, label %body, label %done
body:
  %added = add i8 %sum, %index
  %next = add i8 %index, 1
  br label %loop
done:
  %result = zext i8 %sum to i32
  ret i32 %result
}
