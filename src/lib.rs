use std::io;

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

#[derive(Debug, Clone, Copy)]
pub struct Request {
    uri: StackStr<1024>,
}

impl Request {
    pub fn new(uri: impl AsRef<str>) -> Option<Self> {
        let uri = uri.as_ref();
        if uri.len() > 1024 || uri.starts_with('\u{FEFF}') {
            return None;
        }
        let view = uri::Uri::new(uri).ok()?;
        // SEE: 1.2 Gemini URI scheme
        if view.host.is_none() || view.userinfo.is_some() {
            return None;
        };
        Some(Self {
            uri: uri.try_into().ok()?,
        })
    }

    pub fn read<R: std::io::Read>(_reader: R) -> Option<Self> {
        unimplemented!();
    }
    pub fn write<W: std::io::Write>(_writer: W) -> Option<Self> {
        unimplemented!();
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
}

pub mod status {
    #[derive(Debug, thiserror::Error)]
    #[error("Status code is not within the acceptable range: {0}")]
    pub struct InvalidStatusError(u8);

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

pub mod uri {
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error("URI failed to validate")]
        Invalid,
    }

    #[derive(Debug, Clone)]
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
                    if scheme.chars().all(Self::is_scheme) {
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

        fn is_scheme(c: char) -> bool {
            c.is_alphabetic() || c.is_ascii_digit() || "+-.".contains(c)
        }
        fn is_unreserved(c: char) -> bool {
            "-._~".contains(c) || c.is_ascii_digit() || c.is_alphabetic()
        }
        fn is_path(c: char) -> bool {
            Self::is_unreserved(c) || "%!$&'()*+,;=:@/".contains(c)
        }
        fn is_query(c: char) -> bool {
            c == '?' || Self::is_path(c)
        }
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
            assert_eq!(percent_decode("%21%40%23%24%25%2A%28%29With Some Text in the middle%7E%7B%7D%3A%3C%3E%3F_%2B").unwrap(), "!@#$%*()With Some Text in the middle~{}:<>?_+");
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
