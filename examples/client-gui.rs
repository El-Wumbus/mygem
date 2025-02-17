use eframe::egui;
use egui::{Color32, Key, PointerButton, Rgba, RichText, Ui};
use mygem::{
    uri::{Uri, UriOwned},
    *,
};
use std::sync::{Arc, Mutex, mpsc::channel};

struct State {
    page_content: String,
    processing: bool,
    /// Navigation stack for *simple* back functionalility
    nav: Vec<UriOwned>,
}

fn main() -> eframe::Result {
    let (sender, receiver) = channel::<()>();
    let state = Arc::new(Mutex::new(State {
        page_content: String::new(),
        processing: false,
        nav: Vec::new(),
    }));

    std::thread::spawn({
        let state = state.clone();
        move || {
            let client = Client::new();
            while receiver.recv().is_ok() {
                let mut req_url = {
                    let mut state = state.lock().unwrap();
                    let Some(req_url) = state.nav.last() else {
                        continue;
                    };
                    let req_url = req_url.to_string();
                    state.processing = true;
                    req_url
                };

                let page_content;
                loop {
                    if let Ok(request) = Request::new(&req_url) {
                        match client.send_request(request) {
                            Ok(response)
                                if response.header.status == Status::Success
                                    && response.header.meta().starts_with("text/") =>
                            {
                                page_content =
                                    response.body_as_str().unwrap().to_string();
                                break;
                            }
                            Ok(resp)
                                if matches!(resp.header.status, Status::Redirect(_)) =>
                            {
                                req_url = resp.header.meta().to_string();
                                eprintln!("Following redirect to \"{}\"", req_url);
                                continue;
                            }
                            Ok(response) => {
                                page_content = format!(
                                    "{:?}: {}",
                                    response.header.status,
                                    response.header.meta()
                                );
                                break;
                            }
                            Err(e) => {
                                page_content = format!(
                                    "Failed to make request to \"{}\"; {e}",
                                    request.url_as_str()
                                );
                                break;
                            }
                        }
                    } else {
                        page_content = "Invalid request URL!".to_string();
                        break;
                    };
                }

                let mut state = state.lock().unwrap();
                state.page_content = page_content;
                state.processing = false;
            }
        }
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };

    // Our application state:
    let mut search_bar_text = "".to_owned();

    eframe::run_simple_native("Gemini Client", options, move |ctx, _frame| {
        let mut state = state.lock().unwrap();
        egui::TopBottomPanel::top("Search").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("back").clicked() && state.nav.len() >= 2 {
                    assert!(state.nav.pop().is_some());
                    search_bar_text = state.nav.last().unwrap().to_string();
                    sender.send(()).unwrap();
                }
                ui.text_edit_singleline(&mut search_bar_text);
                ctx.input(|i| {
                    if i.key_pressed(Key::Enter) {
                        if let Ok(url) =
                            Uri::new(search_bar_text.trim()).map(UriOwned::from)
                        {
                            state.nav.push(url);
                            sender.send(()).unwrap();
                        } else {
                            eprintln!("Url is invalid!");
                        }
                    }
                    if i.pointer.button_clicked(PointerButton::Extra1)
                        && state.nav.len() >= 2
                    {
                        assert!(state.nav.pop().is_some());
                        search_bar_text = state.nav.last().unwrap().to_string();
                        sender.send(()).unwrap();
                    }
                });
                if state.processing {
                    ui.add(egui::Spinner::new());
                }
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Some(navto) = render_gemtext(
                    ui,
                    Gemtext::new(&state.page_content),
                    state.nav.last(),
                ) {
                    search_bar_text = navto.to_string();
                    state.nav.push(navto);
                    sender.send(()).unwrap();
                }
            });
        });
    })
}

/// Optionally returns a url to navigate to. This handles rendered links.
fn render_gemtext(
    ui: &mut Ui,
    gemtext: Gemtext,
    last_path: Option<&UriOwned>,
) -> Option<UriOwned> {
    let mut navto = None;
    for line in gemtext {
        match line {
            GemtextToken::Text(text, pre) => {
                if pre.preformatted {
                    ui.label(RichText::new(text).monospace().code());
                } else {
                    ui.label(text);
                }
            }
            GemtextToken::Heading(text, _level) => {
                ui.label(RichText::new(text).heading());
            }
            GemtextToken::List(text, indentation) => {
                ui.label(format!("{}• {text}", " ".repeat(indentation as usize)));
            }
            GemtextToken::Quote(text) => {
                ui.label(
                    RichText::new(format!("> {text}"))
                        .background_color(Rgba::from(Color32::DARK_BLUE).multiply(0.3)),
                );
            }
            GemtextToken::Link(link, text) => {
                // Pages may use relative links which aren't valid URLs, so these must be
                // corrected.
                let Ok(url) = Uri::new(link) else {
                    ui.label(link);
                    continue;
                };
                if url.host.is_none() {
                    let mut url = UriOwned::from(url);
                    let (mut path, dir) = if let Some(current_path) = last_path {
                        url.host = current_path.host.clone();
                        let p = current_path.path.as_deref().unwrap_or("/");
                        (std::path::PathBuf::from(p), p.ends_with('/'))
                    } else {
                        (std::path::PathBuf::new(), true)
                    };
                    if let Some(p) =
                        url.path.as_deref().map(|x| x.trim_start_matches('/'))
                    {
                        if !dir {
                            path.pop();
                        }
                        path.push(p);
                    }
                    url.path = Some(path.to_str().unwrap().to_string());
                    url.scheme = url.scheme.or_else(|| Some("gemini".to_string()));
                    if match text {
                        Some(text) => ui.link(text),
                        None => ui.link(link),
                    }
                    .clicked()
                    {
                        navto = Some(url);
                    };
                } else if match text {
                    Some(text) => ui.link(text),
                    None => ui.link(link),
                }
                .clicked()
                {
                    navto = Some(url.into());
                }
            }
        }
    }
    navto
}
