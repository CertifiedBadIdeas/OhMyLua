enum Command {
    Stop,
    Pause,
    GoTo { x: i32, y: i32 },
    SetThrottle(i32),
}

fn dispatch(command: Command) -> i32 {
    match command {
        Command::Stop | Command::Pause => 0,
        Command::GoTo { x, y } => x + y,
        Command::SetThrottle(t) => t * 2,
    }
}

fn guarded(command: Command, gate: bool) -> i32 {
    match command {
        Command::Stop if gate => 1,
        Command::Stop => 0,
        _ => 2,
    }
}

fn main() {
    let _ = dispatch(Command::Stop);
    let _ = dispatch(Command::GoTo { x: 20, y: 22 });
    let _ = dispatch(Command::SetThrottle(21));
    let _ = guarded(Command::Pause, true);
}
