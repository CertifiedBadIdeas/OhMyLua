struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn sum(&self) -> i32 {
        self.x + self.y
    }
}

fn verify(point: &Point) -> i32 {
    point.sum()
}

fn main() {
    let point = Point { x: 20, y: 22 };
    let _answer = verify(&point);
}
