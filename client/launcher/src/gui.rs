use serde_json::{Value, json};
use skhid_launcher::{
    runtime,
    skin::Skin,
    update::{self, Layout},
};
use std::{borrow::Cow, path::PathBuf};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use wry::{WebView, WebViewBuilder};

#[derive(Debug)]
enum Message {
    Command(Value),
    Progress(update::Progress),
    GameStarted,
    Done(String, Result<String, String>),
    Page(u64, String),
}
struct State {
    folder: PathBuf,
    site: String,
    language: String,
    busy: bool,
    page_id: u64,
}
fn send(view: &WebView, value: Value) {
    if let Err(e) = view.evaluate_script(&format!("window.receive({value})")) {
        eprintln!("UI update failed: {e}");
    }
}
fn code(language: &str) -> &'static str {
    match language {
        "russian" => "ru",
        "ukrainian" => "ua",
        "german" => "de",
        "polish" => "pl",
        _ => "en",
    }
}
fn trusted(uri: &str) -> bool {
    uri == "skhid://localhost/index.html" || uri == "http://skhid.localhost/index.html"
}
fn external(uri: &str) {
    if uri.starts_with("https://")
        || uri.starts_with("http://127.0.0.1:")
        || uri.starts_with("http://localhost:")
    {
        let _ = open::that_detached(uri);
    }
}
fn local_page(layout: &Layout, language: &str, loading: bool) -> String {
    let code = code(language);
    std::fs::read_to_string(layout.bin.join("launcher_skhid").join(format!("index-{}{code}.html",if loading {"blank-"} else {""})))
        .unwrap_or_else(|_| format!("<!doctype html><html><body style='background:#161514;color:#939392;font:13px Arial;text-align:center;padding-top:130px'>{}</body></html>",
        match (language,loading) {
            ("ukrainian",true)=>"Завантаження…",("ukrainian",false)=>"Сторінка сервера недоступна.",
            ("russian",true)=>"Загрузка…",("russian",false)=>"Страница сервера недоступна.",
            ("german",true)=>"Laden…",("german",false)=>"Die Serverseite ist nicht verfügbar.",
            ("polish",true)=>"Ładowanie…",("polish",false)=>"Strona serwera jest niedostępna.",
            (_,true)=>"Loading…",(_,false)=>"The server page is unavailable.",
        }))
}
impl State {
    fn refresh(&mut self, view: &WebView, proxy: &EventLoopProxy<Message>) {
        let result = (|| {
            let layout = Layout::new(&self.folder)?;
            let skin = Skin::load(&layout)?;
            send(
                view,
                json!({"skin":skin,"folder":self.folder,"site":self.site,"language":self.language,"linux":cfg!(target_os="linux"),"busy":self.busy,"error":""}),
            );
            self.page_id += 1;
            let id = self.page_id;
            send(
                view,
                json!({"page":local_page(&layout,&self.language,true)}),
            );
            let fallback = local_page(&layout, &self.language, false);
            let site = self.site.clone();
            let language = self.language.clone();
            let proxy = proxy.clone();
            std::thread::spawn(move || {
                let page = update::fetch(
                    &site,
                    &format!("/launcher/v2/survarium/?lang={}", code(&language)),
                    2 * 1024 * 1024,
                    &mut |_| {},
                )
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or(fallback);
                let _ = proxy.send_event(Message::Page(id, page));
            });
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            send(
                view,
                json!({"folder":self.folder,"site":self.site,"language":self.language,"linux":cfg!(target_os="linux"),"phase":"error","error":e}),
            );
        }
    }
    fn start(&mut self, action: &str, view: &WebView, proxy: &EventLoopProxy<Message>) {
        if self.busy {
            return;
        }
        let prepared = (|| {
            let layout = Layout::new(&self.folder)?;
            if action == "update" {
                update::validate_site(&self.site)?;
            }
            runtime::save_language(&layout, &self.language)?;
            Ok::<_, String>(layout)
        })();
        let layout = match prepared {
            Ok(l) => l,
            Err(e) => {
                send(view, json!({"phase":"error","error":e}));
                return;
            }
        };
        self.busy = true;
        send(
            view,
            json!({"busy":true,"phase":if action=="update" {"checking"} else {action},"percent":0,"detail":"","error":""}),
        );
        let proxy = proxy.clone();
        let site = self.site.clone();
        let language = self.language.clone();
        let action = action.to_owned();
        std::thread::spawn(move || {
            let operation = || match action.as_str() {
                "update" => update::check_progress(&layout, &site, &mut |p| {
                    let _ = proxy.send_event(Message::Progress(p));
                }),
                "setup" => runtime::run(&layout, &language, true),
                _ => {
                    #[cfg(target_os = "linux")]
                    runtime::run(&layout, &language, true)?;
                    let _ = proxy.send_event(Message::GameStarted);
                    runtime::run(&layout, &language, false)
                }
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                .unwrap_or_else(|_| Err("The operation stopped unexpectedly. Restart the launcher to recover the update.".into()));
            let _ = proxy.send_event(Message::Done(action, result));
        });
    }
}
pub fn run(folder: PathBuf, site: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let layout = Layout::new(&folder).ok();
    let mut state = State {
        folder,
        site: site
            .or_else(|| layout.as_ref().and_then(|l| l.site().ok()))
            .unwrap_or_else(|| "https://skhid.link".into()),
        language: layout
            .as_ref()
            .map(runtime::load_language)
            .unwrap_or_else(|| "english".into()),
        busy: false,
        page_id: 0,
    };
    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Survarium — Skhid")
        .with_inner_size(LogicalSize::new(899.0, 549.0))
        .with_resizable(false)
        .with_decorations(false)
        .build(&event_loop)?;
    let ipc = proxy.clone();
    let builder=WebViewBuilder::new()
        .with_custom_protocol("skhid".into(),|_,request| {
            let (status,body)=if request.uri().path()=="/index.html" {(200,include_bytes!("../ui/index.html").as_slice())} else {(404,b"Not found".as_slice())};
            wry::http::Response::builder().status(status).header("Content-Type","text/html; charset=utf-8")
                .header("Content-Security-Policy","default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: https:; frame-src 'self' about:; font-src data:")
                .body(Cow::Borrowed(body)).unwrap()
        })
        .with_url("skhid://localhost/index.html")
        .with_ipc_handler(move |request| {
            // No IPC from server content. That content also has a scriptless, opaque-origin sandbox.
            if trusted(&request.uri().to_string()) && request.body().len()<16384 {
                if let Ok(value)=serde_json::from_str(request.body()) {let _=ipc.send_event(Message::Command(value));}
            }
        })
        .with_navigation_handler(|uri| trusted(&uri)||uri=="about:blank"||uri=="about:srcdoc")
        .with_new_window_req_handler(|uri,_| {external(&uri);wry::NewWindowResponse::Deny});
    #[cfg(target_os = "linux")]
    let view = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().ok_or("Missing GTK container")?)?
    };
    #[cfg(not(target_os = "linux"))]
    let view = builder.build(&window)?;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if !state.busy {
                    *flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(Message::Command(value)) => {
                let action = value["action"].as_str().unwrap_or("");
                match action {
                    "ready" => {
                        state.refresh(&view, &proxy);
                        if Layout::new(&state.folder).is_ok() {
                            state.start("update", &view, &proxy);
                        }
                    }
                    "close" => {
                        if !state.busy {
                            *flow = ControlFlow::Exit;
                        }
                    }
                    "minimize" => window.set_minimized(true),
                    "drag" => {
                        let _ = window.drag_window();
                    }
                    "website" | "shop" => {
                        if let Ok(site) = update::validate_site(&state.site) {
                            external(&format!(
                                "{site}/{}?lang={}",
                                if action == "shop" { "exchange" } else { "" },
                                code(&state.language)
                            ));
                        }
                    }
                    "configure" if !state.busy => {
                        if let (Some(folder), Some(site)) =
                            (value["folder"].as_str(), value["site"].as_str())
                        {
                            match update::validate_site(site) {
                                Ok(site) => {
                                    state.folder = folder.into();
                                    state.site = site;
                                    state.refresh(&view, &proxy);
                                    state.start("update", &view, &proxy);
                                }
                                Err(e) => send(&view, json!({"error":e})),
                            }
                        }
                    }
                    "language" if !state.busy => {
                        if let Some(language) = value["language"]
                            .as_str()
                            .filter(|v| runtime::LANGUAGES.iter().any(|(_, l)| l == v))
                        {
                            if let Ok(layout) = Layout::new(&state.folder) {
                                match runtime::save_language(&layout, language) {
                                    Ok(()) => {
                                        state.language = language.into();
                                        state.refresh(&view, &proxy);
                                    }
                                    Err(e) => send(&view, json!({"error":e})),
                                }
                            }
                        }
                    }
                    "play" | "update" | "setup" => state.start(action, &view, &proxy),
                    _ => {}
                }
            }
            Event::UserEvent(Message::Progress(p)) => {
                let mut value = serde_json::to_value(&p).unwrap();
                if let update::Progress::Download {
                    received,
                    total,
                    bytes_per_second,
                } = p
                {
                    value["percent"] = json!(received as f64 / total.max(1) as f64 * 100.0);
                    value["detail"] = json!(format!(
                        "{:.1} / {:.1} MiB — {:.1} MiB/s",
                        received as f64 / 1048576.0,
                        total as f64 / 1048576.0,
                        bytes_per_second as f64 / 1048576.0
                    ));
                }
                send(&view, value);
            }
            Event::UserEvent(Message::GameStarted) => window.set_visible(false),
            Event::UserEvent(Message::Done(action, result)) => {
                state.busy = false;
                match result {
                    Ok(_) => {
                        if action == "play" {
                            *flow = ControlFlow::Exit;
                            return;
                        }
                        send(
                            &view,
                            json!({"busy":false,"phase":"ready","percent":100,"detail":""}),
                        );
                        if action == "update" {
                            state.refresh(&view, &proxy);
                        }
                    }
                    Err(e) => {
                        window.set_visible(true);
                        send(
                            &view,
                            json!({"busy":false,"phase":"error","error":e,"detail":""}),
                        );
                    }
                }
            }
            Event::UserEvent(Message::Page(id, page)) if id == state.page_id => {
                send(&view, json!({"page":page}));
            }
            _ => {}
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_local_ui_can_issue_commands() {
        assert!(trusted("skhid://localhost/index.html"));
        assert!(trusted("http://skhid.localhost/index.html"));
        for uri in [
            "https://skhid.link/",
            "about:srcdoc",
            "null",
            "skhid://localhost/other",
            "http://skhid.localhost.evil/index.html",
        ] {
            assert!(!trusted(uri));
        }
    }
}
