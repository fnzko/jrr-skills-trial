use std::io;

fn main() -> io::Result<()> {
    miniq::serve(io::stdin().lock(), io::stdout().lock())
}
