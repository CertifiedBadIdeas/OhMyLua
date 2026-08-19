struct Wrapper<T> {
    value: T,
}

fn main() {
    let _wrapped = Wrapper { value: 42_i32 };
}
