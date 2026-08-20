enum Command {
    Stop,
    Go,
}

fn inspect(command: &Command) -> i32 {
    match *command {
        Command::Stop => 0,
        Command::Go => 1,
    }
}

fn main() {
    let stop = Command::Stop;
    let go = Command::Go;
    let _ = inspect(&stop) + inspect(&go);
}
