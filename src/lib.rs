use std::io::{self, Read};
use std::str::Lines;
use std::sync::Arc;

pub use status::Status;

#[derive(Clone, Copy)]
struct StackStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> std::fmt::Debug for StackStr<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.as_str().fmt(f)
    }
}

impl<const N: usize> StackStr<N> {
    fn from_str(s: &str) -> Option<Self> {
        let mut buf = [0u8; N];
        if s.len() > N {
            return None;
        }
        buf[0..s.len()].copy_from_slice(&s.as_bytes()[0..s.len()]);
        Some(Self { buf, len: s.len() })
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[0..self.len]).unwrap()
    }
}

impl<const N: usize> TryFrom<&str> for StackStr<N> {
    type Error = &'static str;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s).ok_or("string is too long")
    }
}

impl<const N: usize> AsRef<str> for StackStr<N> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> std::ops::Deref for StackStr<N> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResponseHeaderParseError {
    #[error("Failed to parse invalid response: {0}")]
    Malformed(&'static str),
    #[error("Failed to parse response: {0}")]
    Status(#[from] status::InvalidStatusError),
}

#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("URL was longer than 1024 bytes")]
    UrlTooLong,
    #[error("URL is not a valid gemini URI")]
    InvalidUrl,
}

#[derive(Debug, Clone, Copy)]
pub struct Request {
    uri: StackStr<1024>,
}

impl Request {
    pub fn new(uri: impl AsRef<str>) -> Result<Self, RequestError> {
        let uri = uri.as_ref();
        if uri.len() > 1024 {
            return Err(RequestError::UrlTooLong);
        }
        let view = uri::Uri::new(uri).map_err(|_| RequestError::UrlTooLong)?;
        // SEE: 1.2 Gemini URI scheme
        if uri.starts_with('\u{FEFF}') || view.host.is_none() || view.userinfo.is_some() {
            return Err(RequestError::InvalidUrl);
        };
        Ok(Self {
            uri: uri.try_into().expect("I checked the length"),
        })
    }
    pub fn url(&self) -> uri::Uri {
        uri::Uri::new(self.uri.as_str()).unwrap()
    }
    pub fn url_as_str(&self) -> &str {
        self.uri.as_str()
    }
    pub fn read<R: std::io::Read>(_reader: R) -> Option<Self> {
        unimplemented!();
    }
    pub fn write<W: std::io::Write>(&self, mut writer: W) -> Result<(), RequestError> {
        writer.write_all(self.uri.as_bytes())?;
        writer.write_all(b"\r\n")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResponseHeader {
    pub status: Status,
    meta: StackStr<1024>,
}

impl ResponseHeader {
    pub fn new(status: Status, meta: &str) -> Result<Self, &'static str> {
        Ok(Self {
            status,
            meta: StackStr::from_str(meta)
                .ok_or("meta can be no longer than 1024 bytes")?,
        })
    }
    pub fn parse(src: impl AsRef<[u8]>) -> Result<Self, ResponseHeaderParseError> {
        let src = src.as_ref();
        let src = std::str::from_utf8(src)
            .map_err(|_| ResponseHeaderParseError::Malformed("is not valid UTF-8"))?;
        if src.starts_with('\u{FEFF}') {
            return Err(ResponseHeaderParseError::Malformed(
                "header starts with U+FEFF",
            ));
        }

        // TODO: do we want to expect this here?
        // we disregard what comes after the CRLF because that's the actual
        // response, but we probably shouldn't get that anyway.
        let (src, _) =
            src.split_once("\r\n")
                .ok_or(ResponseHeaderParseError::Malformed(
                    "does not end with a CR/LF",
                ))?;
        let (status, meta) =
            src.split_once(' ')
                .ok_or(ResponseHeaderParseError::Malformed(
                    "missing space (0x20) separator",
                ))?;
        let status = status
            .parse::<u8>()
            .map_err(|_| ResponseHeaderParseError::Malformed("invalid status code"))?;
        let status = status::Status::try_from(status)?;

        if meta.len() > 1024 {
            return Err(ResponseHeaderParseError::Malformed(
                "META is be longer than 1024 bytes",
            ));
        } else if meta.starts_with('\u{FEFF}') {
            return Err(ResponseHeaderParseError::Malformed(
                "META starts with U+FEFF",
            ));
        }

        Ok(Self {
            status,
            meta: StackStr::from_str(meta)
                .expect("We checked that `meta` fits within 1024"),
        })
    }

    pub fn meta(&self) -> &str {
        self.meta.as_ref()
    }

    pub fn status(&self) -> Status {
        self.status
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResponseReadError {
    #[error("header: {0}")]
    HeaderParse(#[from] ResponseHeaderParseError),
    #[error("IO: {0}")]
    Io(#[from] io::Error),
    #[error("Couldn't parse a response header as there was nothing to parse")]
    MissingHeader,
}

#[derive(Debug)]
pub struct Response {
    pub header: ResponseHeader,
    pub body: Vec<u8>,
}

impl Response {
    pub fn read<R: io::Read>(reader: R) -> Result<Self, ResponseReadError> {
        let mut header = None;
        let mut buffer = Vec::new();
        let mut saw_cr = false;

        for byte in reader.bytes() {
            let byte = byte?;
            buffer.push(byte);

            if header.is_none() {
                if saw_cr && byte == b'\n' {
                    // We're done with the header.
                    header = Some(ResponseHeader::parse(&buffer)?);
                    buffer.clear();
                }
                saw_cr = byte == b'\r';
            }
        }
        let header = header.ok_or(ResponseReadError::MissingHeader)?;

        Ok(Self {
            header,
            body: buffer,
        })
    }

    pub fn body_as_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.body)
    }
}

pub mod status {
    #[derive(Debug, thiserror::Error)]
    #[error("Status code is not within the acceptable range: {0}")]
    pub struct InvalidStatusError(u8);

    // TODO: flatten this structure?
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum Status {
        Input(Input),
        Success,
        Redirect(Redirect),
        TemporaryFailure(TemporaryFailure),
        PermanentFailure(PermanentFailure),
        ClientCertificateRequired(ClientCertificateRequired),
    }

    impl TryFrom<u8> for Status {
        type Error = InvalidStatusError;

        fn try_from(code: u8) -> Result<Self, Self::Error> {
            let x = match code {
                10 => Self::Input(Input::default()),
                11 => Self::Input(Input::Sensitive),
                20 => Self::Success,
                30 => Self::Redirect(Redirect::Temporary),
                31 => Self::Redirect(Redirect::Permanent),
                40 => Self::TemporaryFailure(TemporaryFailure::default()),
                41 => Self::TemporaryFailure(TemporaryFailure::ServerUnavailable),
                42 => Self::TemporaryFailure(TemporaryFailure::CgiError),
                43 => Self::TemporaryFailure(TemporaryFailure::ProxyError),
                44 => Self::TemporaryFailure(TemporaryFailure::SlowDown),
                50 => Self::PermanentFailure(PermanentFailure::default()),
                51 => Self::PermanentFailure(PermanentFailure::NotFound),
                52 => Self::PermanentFailure(PermanentFailure::Gone),
                53 => Self::PermanentFailure(PermanentFailure::ProxyRequestRefused),
                59 => Self::PermanentFailure(PermanentFailure::BadRequest),
                60 => {
                    Self::ClientCertificateRequired(ClientCertificateRequired::default())
                }
                61 => Self::ClientCertificateRequired(
                    ClientCertificateRequired::CertificateNotAuthorized,
                ),
                62 => Self::ClientCertificateRequired(
                    ClientCertificateRequired::CertificateNotValid,
                ),
                _ => {
                    return Err(InvalidStatusError(code));
                }
            };
            Ok(x)
        }
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq)]
    pub enum Input {
        #[default]
        Input,
        Sensitive,
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq)]
    pub enum Redirect {
        #[default]
        Temporary,
        /// The requested resource should be consistently requested from the new
        /// URL provided in future.
        Permanent,
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq)]
    pub enum TemporaryFailure {
        #[default]
        TemporaryFailure,
        /// The server is unavailable due to overload or maintenance. (cf HTTP
        /// 503)
        ServerUnavailable,
        /// A CGI process, or similar system for generating dynamic content,
        /// died unexpectedly or timed out.
        CgiError,
        /// A proxy request failed because the server was unable to successfully
        /// complete a transaction with the remote host. (cf HTTP 502, 504)
        ProxyError,
        /// Rate limiting is in effect.
        SlowDown,
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq)]
    pub enum PermanentFailure {
        #[default]
        PermanentFailure,
        /// The requested resource could not be found but may be available in
        /// the future.
        NotFound,
        /// The resource requested is no longer available and will not be
        /// available again.
        Gone,
        /// The request was for a resource at a domain not served by the server
        /// and the server does not accept proxy requests.
        ProxyRequestRefused,
        /// The server was unable to parse the client's request, presumably due
        /// to a malformed request. (cf HTTP 400)
        BadRequest,
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq)]
    pub enum ClientCertificateRequired {
        #[default]
        ClientCertificateRequired,
        /// The supplied client certificate is not authorised for accessing the
        /// particular requested resource.
        CertificateNotAuthorized,
        /// The supplied client certificate was not accepted because it is not
        /// valid.
        CertificateNotValid,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum GemtextToken<'a> {
    Text(&'a str, TokenPreformatted<'a>),
    /// A link line, where `0` is the the url and `1` is the optional dipslay
    /// name.
    ///
    /// ```not_rust
    /// =>[<whitespace>]<URL>[<whitespace><USER-FRIENDLY LINK NAME>]
    /// ```
    Link(&'a str, Option<&'a str>),
    /// Preformatted text. This may contain multiple lines.
    /// '0' is the text and '1' is the the optional alt-text found after the
    /// first `` ``` ``.
    Preformatted(&'a str, Option<&'a str>),
    /// A heading line. Any line starting with one to three `#` characters.
    /// `0` is the heading text and `1` is the level (or `#` count).
    Heading(&'a str, u8),
    /// A list item. Any line starting with a `*` is a list item.
    List(&'a str, u8),
    /// A quote line. Any line starting with a `>` is a quote line.
    Quote(&'a str),
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TokenPreformatted<'a> {
    pub preformatted: bool,
    pub alt_text: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Gemtext<'a> {
    lines: Lines<'a>,
    pre: TokenPreformatted<'a>,
}

impl<'a> Gemtext<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            lines: src.lines(),
            pre: TokenPreformatted::default(),
        }
    }
}

impl<'a> Iterator for Gemtext<'a> {
    type Item = GemtextToken<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = self.lines.next()?;

        if line.starts_with("```") {
            self.pre.preformatted = !self.pre.preformatted;
            if self.pre.preformatted {
                self.pre.alt_text = Some(line.strip_prefix("```").unwrap().trim_start());
            }
            line = match self.lines.next() {
                Some(x) => x,
                None => {
                    return Some(GemtextToken::Text(line, TokenPreformatted::default()));
                }
            };
        }
        if !self.pre.preformatted && line.starts_with("#") {
            let count = line.chars().filter(|x| *x == '#').count();
            if count < 4 {
                let line =
                    line.trim_start_matches(|x: char| x == '#' || x.is_whitespace());
                return Some(GemtextToken::Heading(line, count as u8));
            }
        } else if !self.pre.preformatted && line.starts_with("=>") {
            let line = line.strip_prefix("=>").unwrap();
            if line.starts_with(char::is_whitespace) {
                let line = line.trim_start();
                let (bruh, moment) = line
                    .split_once(char::is_whitespace)
                    .map(|(x, y)| (x, Some(y.trim_start())))
                    .unwrap_or((line, None));
                return Some(GemtextToken::Link(bruh, moment));
            }
        }
        // TODO: more Gemtext feaures like, preformatted text, list items, and quoted
        // text
        Some(GemtextToken::Text(line, self.pre))
    }
}

pub mod uri {
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error("URI failed to validate")]
        Invalid,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Uri<'a> {
        pub scheme: Option<&'a str>,
        pub userinfo: Option<&'a str>,
        pub host: Option<&'a str>,
        pub port: Option<&'a str>,
        pub path: Option<&'a str>,
        pub query: Option<&'a str>,
        pub fragment: Option<&'a str>,
    }

    impl<'a> Uri<'a> {
        pub fn new(mut src: &'a str) -> Result<Self, Error> {
            let mut uri = Uri {
                scheme: None,
                userinfo: None,
                host: None,
                port: None,
                path: None,
                query: None,
                fragment: None,
            };

            if let Some((rest, frag)) = src.split_once('#') {
                src = rest;
                uri.fragment = Some(frag);
            }
            if let Some((rest, query)) = src.split_once('?') {
                src = rest;
                uri.query = Some(query);
            }

            if src.starts_with(char::is_alphabetic) {
                if let Some((scheme, rest)) = src.split_once(':') {
                    if scheme.chars().all(is_scheme) {
                        uri.scheme = Some(scheme);
                        src = rest;
                    }
                }
            }

            if let Some(rest) = src.strip_prefix("//") {
                src = rest;
                if let Some((rest, path)) = rest.split_once('/') {
                    uri.path = Some(path);
                    src = rest;
                }

                if let Some((rest, port)) = src.rsplit_once(':') {
                    if port.chars().all(|x| x.is_ascii_digit()) {
                        uri.port = Some(port);
                        src = rest;
                    }
                }
                if let Some((userinfo, host)) = src.split_once('@') {
                    uri.userinfo = Some(userinfo);
                    uri.host = Some(host);
                } else {
                    uri.host = Some(src);
                }
            } else {
                uri.path = Some(src);
            }

            Ok(uri)
        }
    }

    impl<'a> From<&'a UriOwned> for Uri<'a> {
        fn from(uri: &'a UriOwned) -> Self {
            Self {
                scheme: uri.scheme.as_deref(),
                userinfo: uri.userinfo.as_deref(),
                host: uri.host.as_deref(),
                port: uri.port.as_deref(),
                path: uri.path.as_deref(),
                query: uri.query.as_deref(),
                fragment: uri.fragment.as_deref(),
            }
        }
    }

    impl ToString for Uri<'_> {
        fn to_string(&self) -> String {
            let mut s = String::new();
            if let Some(scheme) = self.scheme.as_deref() {
                s.push_str(scheme);
                s.push(':');
            }

            if self.host.is_some() {
                s.push_str("//");
                if let Some(userinfo) = self.userinfo.as_deref() {
                    s.push_str(userinfo);
                    s.push('@');
                }
                if let Some(host) = self.host.as_deref() {
                    s.push_str(host);
                }
                if let Some(port) = self.port.as_deref() {
                    s.push(':');
                    s.push_str(port);
                }
                if let Some(path) = self.path.as_deref() {
                    s.push('/');
                    s.push_str(path.trim_start_matches('/'));
                }
            } else if let Some(path) = self.path.as_deref() {
                s.push_str(path);
            }
            if let Some(query) = self.query.as_deref() {
                s.push('?');
                s.push_str(query);
            }
            if let Some(fragment) = self.fragment.as_deref() {
                s.push('#');
                s.push_str(fragment);
            }
            s
        }
    }

    pub struct UriOwned {
        pub scheme: Option<String>,
        pub userinfo: Option<String>,
        pub host: Option<String>,
        pub port: Option<String>,
        pub path: Option<String>,
        pub query: Option<String>,
        pub fragment: Option<String>,
    }

    impl From<Uri<'_>> for UriOwned {
        fn from(uri: Uri) -> Self {
            Self {
                scheme: uri.scheme.map(String::from),
                userinfo: uri.userinfo.map(String::from),
                host: uri.host.map(String::from),
                port: uri.port.map(String::from),
                path: uri.path.map(String::from),
                query: uri.query.map(String::from),
                fragment: uri.fragment.map(String::from),
            }
        }
    }

    impl ToString for UriOwned {
        fn to_string(&self) -> String {
            let uri: Uri = self.into();
            uri.to_string()
        }
    }

    fn is_scheme(c: char) -> bool {
        c.is_alphabetic() || c.is_ascii_digit() || "+-.".contains(c)
    }

    pub fn percent_decode(s: impl AsRef<str>) -> Option<String> {
        let s = s.as_ref();
        let mut out = String::new();
        let mut rem = 0;
        for (i, ch) in s.chars().enumerate() {
            if rem == 0 {
                if ch == '%' {
                    rem = 2;
                } else {
                    out.push(ch);
                }
                continue;
            }
            rem -= 1;
            if rem == 0 {
                out.push(
                    u8::from_str_radix(&s[i - 1..=i], 16)
                        .ok()
                        .and_then(|x| char::try_from(x).ok())?,
                );
            }
        }
        Some(out)
    }

    pub fn percent_encode(_s: impl AsRef<str>) -> Result<String, ()> {
        unimplemented!();
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn percent() {
            assert_eq!(
            percent_decode("%21%40%23%24%25%2A%28%29With Some Text in the middle%7E%7B%7D%3A%3C%3E%3F_%2B").unwrap(),
            "!@#$%*()With Some Text in the middle~{}:<>?_+");
        }

        #[test]
        fn uri() {
            let test1 = "ftp://ftp.is.co.za/rfc/rfc1808.txt";
            let test2 = "http://www.ietf.org/rfc/rfc2396.txt";
            let test3 = "ldap://[2001:db8::7]/c=GB?objectClass?one";
            let test4 = "mailto:John.Doe@example.com";
            let test5 = "news:comp.infosystems.www.servers.unix";
            let test6 = "tel:+1-816-555-1212";
            let test7 = "telnet://192.0.2.16:80/";
            let test8 = "urn:oasis:names:specification:docbook:dtd:xml:4.1.2";
            let test9 = "https://datatracker.ietf.org/doc/html/rfc3986#section-1.1.2";
            let test10 = "https://www.youtube.com/watch?v=QyjyWUrHsFc";
            let test11 =
                "https://john.doe@www.example.com:1234/forum/questions/?query#Frag";
            Uri::new(test1).unwrap();
            Uri::new(test2).unwrap();
            Uri::new(dbg!(test3)).unwrap();
            Uri::new(test4).unwrap();
            Uri::new(test5).unwrap();
            Uri::new(test6).unwrap();
            Uri::new(test7).unwrap();
            let uri2 = Uri::new(test8).unwrap();
            assert_eq!(
                uri2.path.as_deref(),
                Some("oasis:names:specification:docbook:dtd:xml:4.1.2")
            );
            Uri::new(test9).unwrap();
            Uri::new(test10).unwrap();
            let uri = Uri::new(test11).unwrap();
            assert_eq!(uri.scheme, Some("https"));
            assert_eq!(uri.userinfo, Some("john.doe"));
            assert_eq!(uri.host, Some("www.example.com"));
            assert_eq!(uri.port, Some("1234"));
            assert_eq!(uri.path, Some("forum/questions/"));
            assert_eq!(uri.query, Some("query"));
            assert_eq!(uri.fragment, Some("Frag"));
        }

        #[test]
        fn uri_owned() {
            let test1 = "https://www.youtube.com/watch?v=QyjyWUrHsFc";
            let test2 = "http://www.ietf.org/rfc/rfc2396.txt";
            let test3 = "ldap://[2001:db8::7]/c=GB?objectClass?one";
            let test4 = "mailto:John.Doe@example.com";
            let test5 = "news:comp.infosystems.www.servers.unix";
            let test6 = "tel:+1-816-555-1212";
            let test7 = "telnet://192.0.2.16:80/";
            let test8 = "urn:oasis:names:specification:docbook:dtd:xml:4.1.2";

            let uri1 = Uri::new(test1).unwrap();
            assert_eq!(UriOwned::from(dbg!(uri1)).to_string(), test1);
            let uri2 = Uri::new(test2).unwrap();
            assert_eq!(UriOwned::from(dbg!(uri2)).to_string(), test2);
            let uri3 = Uri::new(test3).unwrap();
            assert_eq!(UriOwned::from(dbg!(uri3)).to_string(), test3);
            let uri4 = Uri::new(test4).unwrap();
            assert_eq!(UriOwned::from(dbg!(uri4)).to_string(), test4);
            let uri5 = Uri::new(test5).unwrap();
            assert_eq!(UriOwned::from(dbg!(uri5)).to_string(), test5);
            let uri6 = Uri::new(test6).unwrap();
            assert_eq!(UriOwned::from(dbg!(uri6)).to_string(), test6);
            let uri7 = Uri::new(test7).unwrap();
            assert_eq!(UriOwned::from(dbg!(uri7)).to_string(), test7);
            let uri8 = Uri::new(test8).unwrap();
            assert_eq!(UriOwned::from(dbg!(uri8)).to_string(), test8);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Response: {0}")]
    Response(#[from] ResponseReadError),
    #[error("Rustls: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("Port is invalid")]
    BadPort,
}

pub struct Client {
    cfg: Arc<rustls::client::ClientConfig>,
}

impl Client {
    pub fn new() -> Self {
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DummyVerifier))
            .with_no_client_auth();
        Self {
            cfg: Arc::new(config),
        }
    }

    pub fn send_request(&self, r: Request) -> Result<Response, ClientError> {
        use std::net::TcpStream;
        let url = r.url();
        let host = url.host.unwrap();
        let port = url.port.unwrap_or("1965").parse::<u16>().unwrap();
        let mut cc = rustls::ClientConnection::new(
            self.cfg.clone(),
            ServerName::try_from(host).unwrap().to_owned(),
        )?;
        let mut sock = TcpStream::connect((host, port))?;

        // 1. Request TLS Session
        cc.write_tls(&mut sock).unwrap();
        // 2. Received Server Certificate
        cc.read_tls(&mut sock).unwrap();
        // 3. Check certificate
        cc.process_new_packets().unwrap();
        // 4. Write out request
        r.write(cc.writer()).unwrap();
        // 5. Encrypt request and flush
        cc.write_tls(&mut sock).unwrap();
        let mut closed = false;
        let mut data = Vec::new();
        while !closed {
            while cc.wants_read() && !closed {
                cc.read_tls(&mut sock).unwrap();
                let state = cc.process_new_packets().unwrap();
                closed = state.peer_has_closed();
            }
            let _ = cc.reader().read_to_end(&mut data);
        }

        Ok(Response::read(std::io::Cursor::new(data))?)
    }
}
#[derive(Debug)]
struct DummyVerifier;

use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
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
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn response_header_parse() {
        // https://geminiprotocol.net/docs/tech-overview.gmi
        // 3.1 Response headers
        //
        // > <STATUS><SPACE><META><CR><LF>
        let header = ResponseHeader::parse("20 all good, yup\r\n").unwrap();
        assert_eq!(header.status, Status::Success);
        assert_eq!(header.meta(), "all good, yup");
        assert!(ResponseHeader::parse("59 missing-crlf").is_err());
        assert!(ResponseHeader::parse("59-missing-space\r\n").is_err());
        assert!(ResponseHeader::parse("69 bad number\r\nf").is_err());
        // non UTF-8
        assert!(
            ResponseHeader::parse(b"\xF0\xA4\xAD\xA2\xF0\xA4\xAD\xA2\xF0\xA4\xAD")
                .is_err()
        );

        let meta = "too large <META>".repeat(100);
        assert!(ResponseHeader::parse(format!("20 {meta}\r\n")).is_err());
        let meta = "stuff after a CRLF doesn't get touched\r\n".repeat(50);
        assert!(ResponseHeader::parse(format!("20 {meta}")).is_ok());
    }

    #[test]
    fn response_parse() {
        let text = "20 text/gemini; charset=utf-8\r\nthis is some text\r\n, pretend this is markdown or something".to_string();
        let reader = std::io::BufReader::new(Cursor::new(text.as_bytes().to_vec()));
        let response = Response::read(reader).unwrap();
        assert_eq!(response.header.status, Status::Success);
        assert_eq!(response.header.meta(), "text/gemini; charset=utf-8");
        assert!(std::str::from_utf8(response.body.as_slice()).is_ok());
    }
}
