fn classify(v: i32) -> std::cmp::Ordering {
    if v > 0 {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Less
    }
}

fn main() {
    let _ = classify(1);
}
