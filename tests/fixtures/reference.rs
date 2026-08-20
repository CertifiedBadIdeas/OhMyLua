fn read(value: &i32) -> i32 {
    *value
}

fn main() {
    let value = 1;
    let _value = read(&value);
}
