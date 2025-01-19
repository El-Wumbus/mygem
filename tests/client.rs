use mygem::*;

#[test]
fn main() {
    let c = Client::new();
    let response = c
        .send_request(Request::new("gemini://geminiprotocol.net/").unwrap())
        .unwrap();

    if response.header.status == Status::Success {
        if response.header.meta().starts_with("text/") {
            println!("{}", String::from_utf8_lossy(&response.body));
        } else {
            println!("Binary Data? ({})", response.header.meta());
        }
    } else {
        println!(
            "Server returned status {:?}: {}",
            response.header.status,
            response.header.meta()
        );
    }
}
