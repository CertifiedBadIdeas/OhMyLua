fn arithmetic(left: i32, right: i32) -> i32 {
    let added = left + right;
    let subtracted = added - right;
    let multiplied = subtracted * right;
    let divided = multiplied / right;
    divided % right
}

fn bit_not(value: i32) -> i32 {
    !value
}

fn comparisons(left: i32, right: i32) -> bool {
    let equal = left == right;
    let not_equal = left != right;
    let less = left < right;
    let less_equal = left <= right;
    let greater = left > right;
    let greater_equal = left >= right;
    !(equal == not_equal)
        == ((less == less_equal) == (greater == greater_equal))
}

fn main() {
    let _number = arithmetic(8, 2);
    let _inverted = bit_not(1);
    let _condition = comparisons(1, 2);
}
