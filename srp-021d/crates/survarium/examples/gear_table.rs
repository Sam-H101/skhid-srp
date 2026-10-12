//! Prints each priced 0.21d item: id, name, category, level, price, and the
//! rank each seller needs.
use survarium::dictionary::Dictionary;
use survarium::prices::{price, rank};

fn main() {
    for item in &Dictionary::v021d().items {
        let Some(p) = price(item.dict_id) else { continue };
        let ranks: Vec<String> = p
            .sellers
            .iter()
            .map(|&s| format!("seller{}:rank{}", s, rank(item.dict_id, s)))
            .collect();
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            item.dict_id,
            item.name,
            item.category,
            item.level,
            p.cost,
            ranks.join(",")
        );
    }
}
