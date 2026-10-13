#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use eframe::egui;
use skhid_launcher::{
    runtime,
    update::{self, Layout},
};
use std::{path::PathBuf, sync::mpsc, time::Duration};

enum Event {
    Status(String),
    Done(Result<String, String>),
}
struct Launcher {
    folder: String,
    site: String,
    language: String,
    status: String,
    worker: Option<mpsc::Receiver<Event>>,
    initialized: bool,
}
impl Launcher {
    fn new(folder: PathBuf, site: Option<String>) -> Self {
        let layout = Layout::new(&folder).ok();
        Self {
            folder: folder.to_string_lossy().into_owned(),
            site: site
                .or_else(|| layout.as_ref().and_then(|l| l.site().ok()))
                .unwrap_or_else(|| "https://skhid.link".into()),
            language: layout
                .as_ref()
                .map(runtime::load_language)
                .unwrap_or_else(|| "english".into()),
            status: "Select your Survarium game folder to begin.".into(),
            worker: None,
            initialized: false,
        }
    }
    fn start(&mut self, action: &str) {
        let layout = match Layout::new(&PathBuf::from(&self.folder)) {
            Ok(l) => l,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let site = match if action == "update" {
            update::validate_site(&self.site)
        } else {
            Ok(String::new())
        } {
            Ok(s) => s,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        if let Err(e) = runtime::save_language(&layout, &self.language) {
            self.status = e;
            return;
        }
        let language = self.language.clone();
        let action = action.to_owned();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.status = "Starting…".into();
        std::thread::spawn(move || {
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
                match action.as_str(){
                    "update"=>update::check(&layout,&site,&mut |s|{let _=tx.send(Event::Status(s));}),
                    "setup"=>{let _=tx.send(Event::Status("Preparing Wine and DXVK…".into()));runtime::run(&layout,&language,true)},
                    _=>{let _=tx.send(Event::Status("Game running. First start may install Wine components and DXVK…".into()));runtime::run(&layout,&language,false)}
                }
            })).unwrap_or_else(|_|Err("The operation stopped unexpectedly. Restart the launcher to recover any pending update.".into()));
            let _ = tx.send(Event::Done(result));
        });
    }
}
impl eframe::App for Launcher {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.initialized {
            self.initialized = true;
            if Layout::new(&PathBuf::from(&self.folder)).is_ok() {
                self.start("update");
            }
        }
        let mut done = false;
        if let Some(rx) = &self.worker {
            while let Ok(event) = rx.try_recv() {
                match event {
                    Event::Status(s) => self.status = s,
                    Event::Done(result) => {
                        self.status = result.unwrap_or_else(|e| format!("Could not complete: {e}"));
                        done = true;
                    }
                }
            }
        }
        if done {
            self.worker = None;
        }
        let busy = self.worker.is_some();
        if busy {
            ctx.request_repaint_after(Duration::from_millis(100));
            if ctx.input(|i| i.viewport().close_requested()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            ui.heading("SKHID / SURVARIUM");
            ui.label("Version 0.26g");
            ui.add_space(18.0);
            ui.add_enabled_ui(!busy, |ui| {
                ui.label("Game folder");
                ui.add(egui::TextEdit::singleline(&mut self.folder).desired_width(f32::INFINITY));
                ui.add_space(8.0);
                ui.label("Update server");
                ui.add(egui::TextEdit::singleline(&mut self.site).desired_width(f32::INFINITY));
                ui.add_space(8.0);
                egui::ComboBox::from_label("Game language")
                    .selected_text(
                        runtime::LANGUAGES
                            .iter()
                            .find(|(_, v)| *v == self.language)
                            .map(|(n, _)| *n)
                            .unwrap_or("English"),
                    )
                    .show_ui(ui, |ui| {
                        for (label, value) in runtime::LANGUAGES {
                            ui.selectable_value(&mut self.language, value.into(), label);
                        }
                    });
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized([140.0, 42.0], egui::Button::new("PLAY"))
                        .clicked()
                    {
                        self.start("play");
                    }
                    if ui.button("Check / repair").clicked() {
                        self.start("update");
                    }
                    #[cfg(target_os = "linux")]
                    if ui.button("Set up Wine / DXVK").clicked() {
                        self.start("setup");
                    }
                });
            });
            ui.add_space(16.0);
            ui.separator();
            if busy {
                ui.spinner();
            }
            ui.label(&self.status);
            if busy {
                ui.small("Keep the launcher open until setup finishes or the game closes.");
            }
            ui.add_space(16.0);
            if let Ok(site) = update::validate_site(&self.site) {
                ui.horizontal(|ui| {
                    ui.hyperlink_to("Project website", format!("{site}/"));
                    ui.hyperlink_to("Shop", format!("{site}/exchange"));
                });
            }
            #[cfg(target_os = "linux")]
            ui.small("The launcher runs natively. Wine runs the game; first launch sets up DXVK.");
        });
    }
}
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([650.0, 440.0])
            .with_min_inner_size([520.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Skhid",
        options,
        Box::new(move |_| Ok(Box::new(Launcher::new(folder, site)))),
    )?;
    Ok(())
}
