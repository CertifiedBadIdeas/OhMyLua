trait Operations {
    fn add(left: i32, right: i32) -> i32 {
        left + right
    }
}

struct Calculator;

impl Operations for Calculator {
    fn add(left: i32, right: i32) -> i32 {
        left + right
    }
}

fn main() {
    let _value = Calculator::add(1_i32, 2_i32);
}
