enum Command {
    SetThrottle(i32),
}

fn peek(mut command: Command) {
    match command {
        Command::SetThrottle(ref mut value) => {
            let _ = value;
        }
    }
}

fn main() {
    peek(Command::SetThrottle(1));
}
