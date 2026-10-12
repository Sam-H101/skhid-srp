// dbgrab <resources.db> list [substring]          -> path<TAB>size
// dbgrab <resources.db> get <substring> <out_dir> -> writes matching files (path '/' -> '__')
use resources_db::{Archive, NodeKind};
use std::{env, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut archive = Archive::open(&args[1])?;
    let filter = args.get(3).map(|s| s.as_bytes().to_vec()).unwrap_or_default();
    let matches = |p: &[u8]| filter.is_empty() || p.windows(filter.len()).any(|w| w == filter.as_slice());
    let entries = archive.list();
    match args[2].as_str() {
        "list" => {
            for e in entries.iter().filter(|e| matches(&e.path)) {
                println!("{}\t{}\t{:?}", String::from_utf8_lossy(&e.path), e.uncompressed_size, e.kind);
            }
        }
        "get" => {
            let out = Path::new(&args[4]);
            fs::create_dir_all(out)?;
            for e in entries.iter().filter(|e| matches(&e.path) && matches!(e.kind, NodeKind::File { .. })) {
                let name = String::from_utf8_lossy(&e.path).replace(['/', '\\'], "__");
                fs::write(out.join(&name), archive.read_file(e)?)?;
                println!("{name}");
            }
        }
        _ => eprintln!("unknown command"),
    }
    Ok(())
}
