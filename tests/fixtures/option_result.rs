struct MyErr {
    code: i32,
}

fn classify(v: i32) -> Result<i32, MyErr> {
    if v >= 0 {
        Ok(v * 2)
    } else {
        Err(MyErr { code: v })
    }
}

fn main() {
    let a: Option<i32> = Some(21);
    let b = match a {
        Some(v) => v,
        None => -1,
    };

    let c: Option<i32> = None;
    let d = match c {
        Some(v) => v,
        None => -2,
    };

    let nested: Option<Result<i32, MyErr>> = Some(Ok(7));
    let e = match nested {
        Some(Ok(v)) => v,
        Some(Err(_)) => -3,
        None => -4,
    };

    match classify(b) {
        Ok(v) => {
            let _ = 100 / (v - 42);
        }
        Err(error) => {
            let _ = error.code - (-2);
        }
    }

    let _ = d + e;
}
