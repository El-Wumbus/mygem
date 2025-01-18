use std::io;

#[derive(Debug, thiserror::Error)]
pub enum ResponseHeaderParseError {
    #[error("Failed to parse invalid response: {0}")]
    Malformed(&'static str),
    #[error("Failed to parse response: {0}")]
    Status(#[from] status::InvalidStatusError),
}

#[derive(Debug, Clone, Copy)]
pub struct ResponseHeader {
    pub status: Status,
    meta: ShortStr<1024>,
}

impl ResponseHeader {
    pub fn new(status: Status, meta: &str) -> Result<Self, &'static str> {
        Ok(Self {
            status,
            meta: ShortStr::from_str(meta)
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
            meta: ShortStr::from_str(meta)
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

#[derive(Clone, Copy)]
struct ShortStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> std::fmt::Debug for ShortStr<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.as_str().fmt(f)
    }
}

impl<const N: usize> ShortStr<N> {
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

impl<const N: usize> AsRef<str> for ShortStr<N> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
pub use status::Status;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
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
