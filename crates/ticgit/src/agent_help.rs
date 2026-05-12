pub const MARKDOWN: &str = include_str!("../../../docs/agents.md");

pub fn print() {
    print!("{MARKDOWN}");
}
