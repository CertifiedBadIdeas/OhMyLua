struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn sum(&self) -> i32 {
        self.x + self.y
    }
}

struct Holder {
    point: Point,
}

fn total(holder: &Holder) -> i32 {
    holder.point.sum()
}

fn require_answer(value: i32) {
    let divisor = if value == 42 { 1 } else { 0 };
    let _validated = 42 / divisor;
}

fn main() {
    let point = Point { x: 20, y: 22 };
    let holder = Holder { point };
    let answer = total(&holder);
    require_answer(answer);
}
