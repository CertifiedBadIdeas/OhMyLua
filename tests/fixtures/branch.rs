fn absolute(value: i32) -> i32 {
    if value >= 0 { value } else { -value }
}

fn add(left: i32, right: i32) -> i32 {
    left + right
}

#[allow(dead_code)]
fn unreachable_unsupported(value: i64) -> i64 {
    value
}

fn main() {
    let sum = add(-5, 2);
    let _magnitude = absolute(sum);
}
