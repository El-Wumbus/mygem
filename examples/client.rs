use mygem::*;
use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let url = args.next().expect("Expected URL");
    let mut request = match Request::new(&url) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Invalid request: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("Request: {request:?}");

    let client = Client::new();

    let mut response: Response;
    // Loop to follow redirects
    loop {
        match client.send_request(request) {
            Ok(r) => {
                response = r;
            }
            Err(e) => {
                eprintln!("Failed to get response: {e}");
                std::process::exit(1);
            }
        };
        if matches!(response.header.status, Status::Redirect(_)) {
            eprintln!("Following redirect: {}", response.header.meta());
            request = match Request::new(response.header.meta()) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Invalid request: {e}");
                    std::process::exit(1);
                }
            };
        } else {
            break;
        }
    }

    if response.header.status != Status::Success {
        eprintln!(
            "Recived error response from url: {}\n{:?}: {}",
            request.url_as_str(),
            response.header.status,
            response.header.meta()
        );
        std::process::exit(1);
    }

    let meta = response.header.meta();
    if meta.starts_with("text/") {
        println!("{}", response.body_as_str().expect("expected utf8 text"));
    } else {
        let path =
            std::path::PathBuf::from("/tmp").join(request.url().path.unwrap_or(""));
        eprintln!("Saving data with mimetype '{}' to {:?}", meta, path);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&response.body)
            .expect("failed to write to file!");
    }
}
