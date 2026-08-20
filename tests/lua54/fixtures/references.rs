#[derive(Clone, Copy)]
struct Inner {
    value: i32,
}

#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
    inner: Inner,
}

impl Point {
    fn bump_y(&mut self) {
        self.y += 3;
    }
}

#[derive(Clone, Copy)]
enum Command {
    Stop,
    SetThrottle(i32),
}

fn require(condition: bool) {
    let divisor = if condition { 1 } else { 0 };
    let _ = 100 / divisor;
}

fn set_scalar(value: &mut i32) {
    *value = 42;
}

fn read_scalar(value: &i32) -> i32 {
    *value
}

fn mutate_point(point: &mut Point) {
    point.x += 10;
    point.inner.value = point.x + point.y;

    let x = &mut point.x;
    *x += 5;
}

fn mutate_command(command: &mut Command) {
    match command {
        Command::Stop => {}
        Command::SetThrottle(value) => {
            *value += 1;
        }
    }
}

fn command_value(command: Command) -> i32 {
    match command {
        Command::Stop => -1,
        Command::SetThrottle(value) => value,
    }
}

fn main() {
    let mut scalar = 1;
    set_scalar(&mut scalar);
    require(scalar == 42);

    let mut point = Point {
        x: 1,
        y: 2,
        inner: Inner { value: 0 },
    };
    mutate_point(&mut point);
    point.bump_y();

    require(point.y == 5);

    // Shared scalar field borrow/read.
    let x = read_scalar(&point.x);
    require(x == 16);

    // Nested mutable field stores through &mut Point.
    require(point.inner.value == 13);

    // A Rust Copy aggregate must not alias the original Lua table.
    let original = Point {
        x: 3,
        y: 4,
        inner: Inner { value: 5 },
    };
    let mut copied = original;
    copied.x = 99;
    copied.inner.value = 77;
    require(original.x == 3);
    require(original.inner.value == 5);
    require(copied.x == 99);
    require(copied.inner.value == 77);

    // Mutable enum payload references must address the payload cell itself.
    let mut command = Command::SetThrottle(10);
    mutate_command(&mut command);
    require(command_value(command) == 11);

    // Keep the unit variant alive as well, so both shapes are lowered.
    let stopped = Command::Stop;
    require(command_value(stopped) == -1);
}
