struct Holder {
    value: &'static i32,
}

static VALUE: i32 = 42;

fn main() {
    let _holder = Holder { value: &VALUE };
}
