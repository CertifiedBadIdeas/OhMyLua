enum Command {
    Stop,
}

fn inspect(command: &Command) -> i32 {
    match *command {
        Command::Stop => 0,
    }
}

fn main() {
    let command = Command::Stop;
    let _ = inspect(&command);
}
