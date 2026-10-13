#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod gui;
use skhid_launcher::{
    runtime,
    update::{self, Layout},
};
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut folder = runtime::initial_folder();
    let mut site = None;
    let mut command = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game-dir" => folder = PathBuf::from(args.next().ok_or("--game-dir needs a path")?),
            "--site" => site = Some(args.next().ok_or("--site needs a URL")?),
            "--check" | "--setup" | "--play" => command = Some(arg),
            "--help" => {
                println!(
                    "skhid-launcher [--game-dir PATH] [--site URL] [--check | --setup | --play]\nWithout an action, opens the native desktop launcher."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }
    if let Some(command) = command {
        let layout = Layout::new(&folder)?;
        let result = match command.as_str() {
            "--check" => update::check(
                &layout,
                &match site {
                    Some(site) => site,
                    None => layout.site()?,
                },
                &mut |s| println!("{s}"),
            ),
            "--setup" => runtime::run(&layout, &runtime::load_language(&layout), true),
            _ => runtime::run(&layout, &runtime::load_language(&layout), false),
        }?;
        println!("{result}");
        return Ok(());
    }
    gui::run(folder, site)?;
    Ok(())
}
