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
    let _ = ordering_mask(false, true);
}
