fn main() -> std::io::Result<()> {
    miniq::run(std::io::stdin().lock(), std::io::stdout().lock())
}
