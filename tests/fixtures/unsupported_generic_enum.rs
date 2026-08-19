enum Wrapper<T> {
    Empty,
    Value(T),
}

fn main() {
    let wrapper = Wrapper::<i32>::Value(7);
    let _ = wrapper;
}
