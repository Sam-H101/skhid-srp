//! Prints what the decoder reads from each saved snapshot in a directory.
use match_server::snapshot;

fn main() {
    let dir = std::env::args().nth(1).expect("directory");
    let players: usize = std::env::args().nth(2).map_or(4, |n| n.parse().unwrap());
    let batteries: usize = std::env::args().nth(3).map_or(5, |n| n.parse().unwrap());
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort_by_key(|n| n.split('_').nth(2).and_then(|t| t.parse::<u32>().ok()));
    for name in names {
        let body = std::fs::read(format!("{dir}/{name}")).unwrap();
        match snapshot::decode(&body, players, batteries) {
            None => println!("{name}: undecodable ({} bytes)", body.len()),
            Some(d) => {
                let stats = d.statistics.as_ref().map(|s| {
                    s.iter()
                        .map(|p| {
                            format!(
                                "k{}d{}f{}h{}",
                                p.events[0], p.events[2], p.events[22], p.events[23]
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                });
                println!(
                    "{name}: {} B, points {:?}, kd {:?}, timelimit {:?}, stats {:?}",
                    body.len(),
                    d.team_points,
                    d.kills_deaths,
                    d.timelimit.start,
                    stats
                );
            }
        }
    }
}
