use std::io;

fn main() -> io::Result<()> {
    miniq::run(io::stdin().lock(), io::stdout().lock())
}
