use eframe::egui;
use egui::{Key, Link, RichText};
use mygem::*;
use std::sync::{Arc, Mutex, mpsc::channel};

struct State {
    page_content: String,
    processing: bool,
    current_url: String,
}

fn main() -> eframe::Result {
    let (sender, receiver) = channel::<String>();
    let state = Arc::new(Mutex::new(State {
        page_content: String::new(),
        processing: false,
        current_url: String::new(),
    }));

    std::thread::spawn({
        let state = state.clone();
        move || {
            let client = Client::new();
            while let Ok(req_url) = receiver.recv() {
                let page_content = if let Ok(request) = Request::new(&req_url) {
                    {
                        let mut state = state.lock().unwrap();
                        state.processing = true;
                    }
                    match client.send_request(request) {
                        Ok(response) => {
                            if response.header.status == Status::Success
                                && response.header.meta().starts_with("text/")
                            {
                                let body = response.body_as_str().unwrap();
                                body.to_string()
                            } else {
                                format!(
                                    "{:?}: {}",
                                    response.header.status,
                                    response.header.meta()
                                )
                            }
                        }
                        Err(e) => format!(
                            "Failed to make request to \"{}\"; {e}",
                            request.url_as_str()
                        ),
                    }
                } else {
                    format!("Invalid request URL!")
                };
                let mut state = state.lock().unwrap();
                state.page_content = page_content;
                state.processing = false;
                state.current_url = req_url;
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
        let state = state.lock().unwrap();
        egui::TopBottomPanel::top("Search").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut search_bar_text);
                ctx.input(|i| {
                    if i.key_pressed(Key::Enter) {
                        sender.send(search_bar_text.trim().to_string()).unwrap();
                    }
                });
                if state.processing {
                    ui.add(egui::Spinner::new());
                }
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for line in Gemtext::new(&state.page_content) {
                    match line {
                        GemtextToken::Text(text) => {
                            ui.label(text);
                        }
                        GemtextToken::Heading(text, level) => {
                            ui.label(RichText::new(text).size(16.0 + 4.0 - level as f32));
                        }
                        GemtextToken::Link(link, text) => {
                            // TODO: fix relative links after adding mutable version of
                            // `Url` to library
                            if match text {
                                Some(text) => ui.link(text),
                                None => ui.link(link),
                            }
                            .clicked()
                            {
                                search_bar_text = link.to_string();
                                sender.send(search_bar_text.trim().to_string()).unwrap();
                            };
                        }
                        _ => unreachable!(),
                    }
                }
            });
        });
    })
}
