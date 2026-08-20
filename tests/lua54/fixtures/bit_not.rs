fn invert(value: i32) -> i32 {
    !value
}

fn checked_divisor(condition: bool) -> i32 {
    if condition { 1 } else { 0 }
}

fn main() {
    let first = invert(1);
    let _ = 1 / checked_divisor(first == -2);

    let boundary = invert(-2147483648);
    let _ = 1 / checked_divisor(boundary == 2147483647);
}
