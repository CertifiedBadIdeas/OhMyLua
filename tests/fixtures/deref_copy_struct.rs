#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

fn copy_point(point: &Point) -> Point {
    *point
}

fn main() {
    let point = Point { x: 20, y: 22 };
    let _copy = copy_point(&point);
}
