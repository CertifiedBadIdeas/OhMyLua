fn classify(value: i32) -> i32 {
    if value >= 0 { value } else { -value }
}

fn main() {
    let magnitude = classify(-3);
    assert_eq!(magnitude, 3);
}
