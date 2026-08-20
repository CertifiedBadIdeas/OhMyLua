fn checked_divisor(condition: bool) -> i32 {
    if condition { 1 } else { 0 }
}

fn ordering_mask(left: bool, right: bool) -> i32 {
    let mut mask = 0;
    if left < right {
        mask += 1;
    }
    if left <= right {
        mask += 2;
    }
    if left > right {
        mask += 4;
    }
    if left >= right {
        mask += 8;
    }
    mask
}

fn main() {
    let _ = 1 / checked_divisor(ordering_mask(false, false) == 10);
    let _ = 1 / checked_divisor(ordering_mask(false, true) == 3);
    let _ = 1 / checked_divisor(ordering_mask(true, false) == 12);
    let _ = 1 / checked_divisor(ordering_mask(true, true) == 10);
}
