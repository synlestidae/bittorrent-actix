use std::ascii::AsciiExt;
use std::{num::ParseIntError, string::FromUtf8Error};

#[derive(Debug, Eq, PartialEq)]
pub struct Uri {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: Option<String>,
    pub query: Vec<(String, String)>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ParseErr {
    Encoding,
    UnexpectedEnd { while_parsing: &'static str },
    UnexpectedChar { expected: u8, actual: u8 },
    InvalidPort,
}

impl From<FromUtf8Error> for ParseErr {
    fn from(_err: FromUtf8Error) -> Self {
        Self::Encoding
    }
}

impl From<ParseIntError> for ParseErr {
    fn from(_err: ParseIntError) -> Self {
        Self::InvalidPort
    }
}

impl Uri {
    pub fn parse(uri: &str) -> Result<Self, ParseErr> {
        let mut i = 0;
        let bytes = uri.as_bytes();
        let scheme = Self::parse_scheme(bytes, &mut i)?;
        let host = Self::parse_host(bytes, &mut i)?;
        let port = Self::parse_port(bytes, &mut i)?;
        let path = Self::parse_path(bytes, &mut i)?;
        let query = Self::parse_query(bytes, &mut i)?;

        Ok(Uri {
            scheme,
            host,
            port,
            path,
            query,
        })
    }

    fn parse_scheme(uri: &[u8], i: &mut usize) -> Result<String, ParseErr> {
        let start = *i;
        while *i < uri.len() && uri[*i] != b':' {
            *i += 1;
        }
        let scheme = String::from_utf8(uri[start..*i].to_vec())?;
        Self::eat(uri, i, "://")?;

        Ok(scheme)
    }

    fn parse_host(uri: &[u8], i: &mut usize) -> Result<String, ParseErr> {
        let start = *i;
        while *i < uri.len() && uri[*i] != b':' && uri[*i] != b'/' {
            *i += 1;
        }

        if *i == start {
            return Err(ParseErr::UnexpectedEnd {
                while_parsing: "host",
            });
        }

        let host = String::from_utf8(uri[start..*i].to_vec())?;
        Ok(host)
    }

    fn parse_port(uri: &[u8], i: &mut usize) -> Result<Option<u16>, ParseErr> {
        println!("parse port");
        if *i == uri.len() {
            println!("end of shit");
            return Ok(None);
        }

        if uri[*i] != b':' {
            println!("end of colon");
            return Ok(None);
        }
        Self::eat(uri, i, ":")?;

        let start = *i;
        while *i < uri.len() && uri[*i].is_ascii_digit() {
            *i += 1;
        }
        if *i == start {
            return Err(ParseErr::InvalidPort);
        }

        let digits = String::from_utf8(uri[start..*i].to_vec())?;

        Ok(Some(u16::from_str_radix(&digits, 10)?))
    }

    fn parse_path(uri: &[u8], i: &mut usize) -> Result<Option<String>, ParseErr> {
        let start = *i;
        if *i == uri.len() {
            return Ok(None);
        }
        while *i < uri.len() && uri[*i] != b'?' {
            *i += 1;
        }
        if *i == start {
            return Ok(None);
        }
        let path = String::from_utf8(uri[start..*i].to_vec())?;
        Ok(Some(path))
    }

    fn parse_query(uri: &[u8], i: &mut usize) -> Result<Vec<(String, String)>, ParseErr> {
        let mut query = Vec::new();
        if *i == uri.len() {
            return Ok(vec![]);
        }
        loop {
            if *i == uri.len() {
                return Ok(query);
            }

            let mut key = Vec::new();
            let mut val = Vec::new();

            while *i < uri.len() && uri[*i] != b'=' {
                *i += 1;
                key.push(uri[*i]);
            }

            if *i == uri.len() {
                query.push((String::from_utf8(key)?, String::from_utf8(val)?));
                return Ok(query);
            }
            Self::eat(uri, i, "=")?;

            while *i < uri.len() && uri[*i] != b'&' {
                *i += 1;
                val.push(uri[*i]);
            }

            query.push((String::from_utf8(key)?, String::from_utf8(val)?));

            if *i < uri.len() {
                Self::eat(uri, i, "&");
            }
        }
    }

    fn eat(uri: &[u8], i: &mut usize, expect: &'static str) -> Result<(), ParseErr> {
        let mut j = 0;
        let expect_bytes = expect.as_bytes();

        while j < expect.len() {
            if *i + j >= uri.len() {
                return Err(ParseErr::UnexpectedEnd {
                    while_parsing: expect,
                });
            }
            if expect_bytes[j] != uri[*i + j] {
                return Err(ParseErr::UnexpectedChar {
                    expected: expect_bytes[j],
                    actual: uri[*i + j],
                });
            }
            j += 1;
        }

        *i += j;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::ParseErr;
    use super::Uri;

    #[test]
    fn test_parse_google_homepage() {
        assert_eq!(
            Uri::parse("https://google.com"),
            Ok(Uri {
                scheme: "https".to_owned(),
                host: "google.com".to_owned(),
                port: None,
                path: None,
                query: vec![]
            })
        );
    }

    #[test]
    fn test_parse_udp_bittorrent() {
        assert_eq!(
            Uri::parse("udp://example.com:1999"),
            Ok(Uri {
                scheme: "udp".to_owned(),
                host: "example.com".to_owned(),
                port: Some(1999),
                path: None,
                query: vec![]
            })
        );
    }

    #[test]
    fn test_parse_empty() {
        assert_eq!(
            Uri::parse(""),
            Err(ParseErr::UnexpectedEnd {
                while_parsing: "://"
            })
        );
    }

    #[test]
    fn test_parse_https_with_path() {
        assert_eq!(
            Uri::parse("https://example.com/path/to/resource"),
            Ok(Uri {
                scheme: "https".to_owned(),
                host: "example.com".to_owned(),
                port: None,
                path: Some("/path/to/resource".to_owned()),
                query: vec![]
            })
        );
    }

    #[test]
    fn test_part_localhost_with_port() {
        assert_eq!(
            Uri::parse("http://localhost:1234"),
            Ok(Uri {
                scheme: "http".to_owned(),
                host: "localhost".to_owned(),
                port: Some(1234),
                path: None,
                query: vec![]
            })
        );
    }
}
