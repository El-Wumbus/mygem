use mygem::{Request, Response, Status};
use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

fn main() {
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(DummyVerifier))
        .with_no_client_auth();

    let rc_config = Arc::new(config);
    let server = "localhost".try_into().unwrap();

    let mut client = rustls::ClientConnection::new(rc_config, server).unwrap();

    // TODO: test api
    let request = Request::new("gemini://geminiprotocol.net/").unwrap();
    let mut socket = TcpStream::connect("geminiprotocol.net:1965").unwrap();

    // 1. Request TLS Session
    client.write_tls(&mut socket).unwrap();

    // 2. Received Server Certificate
    client.read_tls(&mut socket).unwrap();

    // 3. Check certificate
    client.process_new_packets().unwrap();

    // 4. Write out request
    request.write(client.writer()).unwrap();

    // 5. Encrypt request and flush
    client.write_tls(&mut socket).unwrap();

    // 6. Decrypt response

    let mut closed = false;
    let mut data = Vec::new();

    while !closed {
        while client.wants_read() {
            client.read_tls(&mut socket).unwrap();
            let state = client.process_new_packets().unwrap();
            if state.peer_has_closed() {
                closed = true;
            }
        }
        client.reader().read_to_end(&mut data);
    }

    let response = Response::read(std::io::Cursor::new(data)).unwrap();
    if response.header.status == Status::Success {
        if response.header.meta().starts_with("text/") {
            println!("{}", String::from_utf8_lossy(&response.body))
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

#[derive(Debug)]
struct DummyVerifier;

use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
impl ServerCertVerifier for DummyVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        use rustls::SignatureScheme::*;
        vec![
            RSA_PKCS1_SHA1,
            ECDSA_SHA1_Legacy,
            RSA_PKCS1_SHA256,
            ECDSA_NISTP256_SHA256,
            RSA_PKCS1_SHA384,
            ECDSA_NISTP384_SHA384,
            RSA_PKCS1_SHA512,
            ECDSA_NISTP521_SHA512,
            RSA_PSS_SHA256,
            RSA_PSS_SHA384,
            RSA_PSS_SHA512,
            ED25519,
            ED448,
        ]
    }
}
