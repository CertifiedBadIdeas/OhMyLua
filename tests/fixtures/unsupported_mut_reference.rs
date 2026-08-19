struct Counter {
    value: i32,
}

impl Counter {
    fn increment(&mut self) {
        self.value += 1;
    }
}

fn main() {
    let mut counter = Counter { value: 0 };
    counter.increment();
}
