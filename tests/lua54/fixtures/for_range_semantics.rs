fn require(condition: bool) {
    let divisor = if condition { 1 } else { 0 };
    let _ = 100 / divisor;
}

fn main() {
    let mut first = -1;
    for i in 2..5 {
        first = i;
        break;
    }
    require(first == 2);

    let mut empty_count = 0;
    for _ in 0..0 {
        empty_count += 1;
    }
    require(empty_count == 0);

    let mut count = 0;
    for _ in 0..1 {
        count += 1;
    }
    require(count == 1);
}
