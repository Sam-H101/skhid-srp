//! Prints the parts of saved 0.26g world snapshots (the match server's
//! `SRP_SNAPSHOT_DIR` dumps) side by side: the mask, each player block and
//! the six sections with their lengths and CRCs, to see where two worlds
//! differ.
//!
//! ```text
//! cargo run -p survarium --example snapdiff -- a.bin b.bin ...
//! ```

use survarium::v026g::match_msgs::players;
use survarium::v026g::match_msgs::snapshot::{Sections, crc};

fn hex(bytes: &[u8], max: usize) -> String {
    let mut out: String = bytes
        .iter()
        .take(max)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > max {
        out.push_str(" ..");
    }
    out
}

fn main() {
    for path in std::env::args().skip(1) {
        let body = match std::fs::read(&path) {
            Ok(body) => body,
            Err(error) => {
                println!("{path}: {error}");
                continue;
            }
        };
        println!("== {path}: {} bytes, CRC {:08x}", body.len(), crc(&body));
        let Some(sections) = Sections::walk(&body) else {
            println!("   does not walk from the front");
            continue;
        };
        println!("   mask {:#b}", sections.active_clients_mask);
        if let Some(blocks) = players::blocks(&body) {
            for block in blocks {
                println!(
                    "   player {}: state {} bytes (CRC {:08x}), animation {:?} bytes",
                    block.place,
                    block.state.len(),
                    crc(block.state),
                    block.animation.map(<[u8]>::len)
                );
                println!("      {}", hex(block.state, 400));
            }
        }
        for (name, section) in [
            ("bullets", sections.bullets),
            ("statistics", sections.statistics),
            ("objects", sections.objects),
            ("physics", sections.physics),
            ("rules", sections.rules),
            ("effects", sections.effects),
        ] {
            println!(
                "   {name}: {} bytes, CRC {:08x}: {}",
                section.len(),
                crc(section),
                hex(section, 160)
            );
        }
    }
}
