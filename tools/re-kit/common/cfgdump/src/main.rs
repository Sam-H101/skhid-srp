use binary_config_parser::tree::ConfigValue;
fn show(v: &ConfigValue, depth: usize, name: &str) {
    let pad = "  ".repeat(depth);
    match v {
        ConfigValue::Named(e) => {
            println!("{pad}{name}:");
            for (k, c) in e { show(c, depth + 1, k); }
        }
        ConfigValue::Indexed(e) => {
            println!("{pad}{name}: [");
            for (i, c) in e.iter().enumerate() { show(c, depth + 1, &i.to_string()); }
            println!("{pad}]");
        }
        other => println!("{pad}{name} = {other:?}"),
    }
}
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let root = ConfigValue::parse(&bytes).unwrap();
    show(&root, 0, "root");
}
