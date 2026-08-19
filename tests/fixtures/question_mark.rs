struct MyErr {
    code: i32,
}

fn fallible(v: i32) -> Result<i32, MyErr> {
    if v > 10 {
        Ok(v * 2)
    } else {
        Err(MyErr { code: v })
    }
}

fn try_result() -> Result<i32, MyErr> {
    let a = fallible(21)?;
    let b = fallible(a)?;
    Ok(b + 1)
}

fn maybe(v: i32) -> Option<i32> {
    if v > 0 {
        Some(v * 2)
    } else {
        None
    }
}

fn try_option() -> Option<i32> {
    let a = maybe(5)?;
    let b = maybe(a)?;
    Some(b - 1)
}

fn main() {
    match try_result() {
        Ok(v) => {
            let _ = 100 / (v - 43);
        }
        Err(error) => {
            let _ = error.code - (-2);
        }
    }
    match try_option() {
        Some(v) => {
            let _ = 100 / (v - 19);
        }
        None => {
            let _ = 0 / 1;
        }
    }
}
