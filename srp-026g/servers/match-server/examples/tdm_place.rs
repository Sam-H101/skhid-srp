//! For one roster place, the item counts whose reading of the state head
//! stays plausible in every saved snapshot of a directory, and each one's
//! `m_is_alive` over time. `tdm_place <directory> <place>`.
use survarium::v026g::match_msgs::players;

fn main() {
    let dir = std::env::args().nth(1).expect("directory");
    let place: u8 = std::env::args().nth(2).expect("place").parse().unwrap();
    let mut files: Vec<(u32, Vec<u8>)> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let time = name.split('_').nth(2)?.parse::<u32>().ok()?;
            Some((time, std::fs::read(format!("{dir}/{name}")).ok()?))
        })
        .collect();
    files.sort_by_key(|file| file.0);
    for items in 0..400 {
        let mut line = String::new();
        let mut ok = true;
        for (_, body) in &files {
            let Some(blocks) = players::blocks(body) else {
                ok = false;
                break;
            };
            let Some(block) = blocks.iter().find(|block| block.place == place) else {
                line.push('-');
                continue;
            };
            match players::PlayerState::read(block.state, items) {
                Some(head)
                    if head.look_pitch.is_finite()
                        && head.look_pitch.abs() < 4.0
                        && head.rotation_y.is_finite()
                        && head.rotation_y.abs() < 8.0
                        && head.effects < 40 =>
                {
                    line.push(if head.alive { 'A' } else { 'd' })
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            println!("items {items:3}: {line}");
        }
    }
    println!(
        "times {:?}",
        files.iter().map(|file| file.0 / 1000).collect::<Vec<_>>()
    );
}
