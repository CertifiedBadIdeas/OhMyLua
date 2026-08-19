struct Operations;

impl Operations {
    fn add(left: i32, right: i32) -> i32 {
        left + right
    }
}

fn main() {
    let _value = Operations::add(1_i32, 2_i32);
}
