struct Vec2 {
    x: i32,
    y: i32,
}

impl Vec2 {
    fn squared(&self) -> i32 {
        self.x * self.x + self.y * self.y
    }
}

enum Direction {
    Up,
    Down,
}

enum Command {
    Stop,
    GoTo { x: i32, y: i32 },
    SetThrottle(i32),
    Move(Vec2),
    Turn(Direction),
}

fn dispatch(command: Command) -> i32 {
    match command {
        Command::Stop => 0,
        Command::GoTo { x, y } => x + y,
        Command::SetThrottle(t) => t * 2,
        Command::Move(ref v) => v.squared(),
        Command::Turn(direction) => match direction {
            Direction::Up => 1,
            Direction::Down => 2,
        },
    }
}

fn main() {
    let a = dispatch(Command::Stop);
    let b = dispatch(Command::GoTo { x: 10, y: 22 });
    let c = dispatch(Command::SetThrottle(21));
    let d = dispatch(Command::Move(Vec2 { x: 3, y: 4 }));
    let e = dispatch(Command::Turn(Direction::Down));
    let f = dispatch(Command::Turn(Direction::Up));
    let total = a + b + c + d + e + f;
    let _ = total + 2147483545;
    let _ = -2147483648 + (total - 102);
}
