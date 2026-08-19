fn main() {
    let mut sum = 0;
    for i in (0..10).step_by(2) {
        sum += i;
    }
    let _ = sum - 20;
}
