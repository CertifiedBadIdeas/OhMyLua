fn main() {
    let mut n = 0;
    let mut sum = 0;
    while n < 10 {
        n += 1;
        if n == 5 {
            continue;
        }
        if n == 8 {
            break;
        }
        sum += n;
    }
    let before = sum;
    let tail = loop {
        if sum > 100 {
            break sum;
        }
        sum += 1;
    };
    let _ = 1000 / (before - 22);
    let _ = 1000 / (tail - 100);
    let _ = sum - 8;
}