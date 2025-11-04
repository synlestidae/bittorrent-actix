use std::{mem, string::FromUtf8Error};

#[derive(Debug, Eq, PartialEq)]
pub struct Uri {
    pub scheme: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub path: Option<String>,
    pub query: Vec<(String, String)>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ParseErr {
    Encoding { while_parsing: &'static str, inner: FromUtf8Error },
    UnexpectedEnd { while_parsing: &'static str },
    UnexpectedChar { expected: &'static str, index_actual: usize, index_expected: usize },
    InvalidPort,
    InvalidUriComponent { string: String, index: usize },
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
        let scheme = match String::from_utf8(uri[start..*i].to_vec()) {
            Ok(s) => s,
            Err(inner) => return Err(ParseErr::Encoding { while_parsing: "scheme", inner }),
        };
        Self::eat(uri, i, ":")?;

        Ok(scheme)
    }

    fn parse_host(uri: &[u8], i: &mut usize) -> Result<Option<String>, ParseErr> {
        let start = *i;
        while *i < uri.len() && uri[*i] != b':' && uri[*i] != b'?' && uri[*i] != b'/' {
            *i += 1;
        }

        if *i == start {
            return Ok(None);
        }

        let host = match String::from_utf8(uri[start..*i].to_vec()) {
            Ok(s) => s,
            Err(inner) => return Err(ParseErr::Encoding { while_parsing: "host", inner }),
        };

        Ok(Some(host))
    }

    fn parse_port(uri: &[u8], i: &mut usize) -> Result<Option<u16>, ParseErr> {
        println!("parse port {}", *i);
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
            return Ok(None);
            //return Err(ParseErr::InvalidPort);
        }

        let digits = match String::from_utf8(uri[start..*i].to_vec()) {
            Ok(d) => d,
            Err(inner) => return Err(ParseErr::Encoding { while_parsing: "port", inner }),
        };
        let port = match u16::from_str_radix(&digits, 10) {
            Ok(n) => n,
            Err(_) => return Err(ParseErr::InvalidPort),
        };
        Ok(Some(port))
    }

    fn parse_path(uri: &[u8], i: &mut usize) -> Result<Option<String>, ParseErr> {
        let start = *i;
        if *i == uri.len() {
            return Ok(None);
        }
        println!("parse_path: {}", *i);
        while *i < uri.len() && uri[*i] != b'?' {
            *i += 1;
        }
        if *i == start {
            println!("no path");
            return Ok(None);
        }
        let path = match String::from_utf8(uri[start..*i].to_vec()) {
            Ok(n) => n,
            Err(inner) => return Err(ParseErr::Encoding { while_parsing: "path", inner }),
        };
        Ok(Some(path))
    }

    fn parse_query(uri: &[u8], i: &mut usize) -> Result<Vec<(String, String)>, ParseErr> {
        let mut query = Vec::new();
        if *i == uri.len() {
            return Ok(vec![]);
        }

        Self::eat(uri, i, "?")?;

        loop {
            if *i == uri.len() {
                return Ok(query);
            }

            let mut key = Vec::new();
            let mut val = Vec::new();

            while *i < uri.len() && uri[*i] != b'=' {
                key.push(uri[*i]);
                *i += 1;
            }
            Self::eat(uri, i, "=");
            let decoded_key = decode_uri_encoding(&key)?;
            let key_string = match String::from_utf8(decoded_key) {
                Ok(s) => s,
                Err(inner) => return Err(ParseErr::Encoding { while_parsing: "query", inner }),
            };

            if *i == uri.len() {
                let decoded_val = decode_uri_encoding(&val)?;
                let value_string = match String::from_utf8(decoded_val) {
                    Ok(s) => s,
                    Err(inner) => return Err(ParseErr::Encoding { while_parsing: "query", inner }),
                };
                query.push((key_string, value_string));
                return Ok(query);
            }
            Self::eat(uri, i, "=")?;

            while *i < uri.len() && uri[*i] != b'&' {
                val.push(uri[*i]);
                *i += 1;
            }

            let decoded_key = decode_uri_encoding(&key)?;
            let decoded_val = decode_uri_encoding(&val)?;

            println!("Me parse key {:?}", decoded_key);
            println!("Me parse val {:?}", decoded_val);

            let key_string = match String::from_utf8(decoded_key) {
                Ok(s) => s,
                Err(inner) => return Err(ParseErr::Encoding { while_parsing: "query", inner }),
            };
            let value_string = match String::from_utf8(decoded_val) {
                Ok(s) => s,
                Err(inner) => return Err(ParseErr::Encoding { while_parsing: "query", inner }),
            };
            println!("{}={}", key_string, value_string);
            query.push((key_string, value_string));

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
                    expected: expect,
                    index_actual: *i,
                    index_expected: j,
                });
            }
            j += 1;
        }

        *i += j;

        Ok(())
    }
}

struct KeyValueParser {
    done: bool,
    decoding: bool,
    parsing_key: bool,
    key: Vec<u8>,
    val: Vec<u8>,
    encoded: Vec<u8>,
}

impl KeyValueParser {
    fn new() -> Self {
        Self {
            done: false,
            decoding: false,
            parsing_key: false,
            key: vec![],
            val: vec![],
            encoded: vec![],
        }
    }

    fn push(&mut self, byte: u8) {
        if byte == b'&' {
            self.done = true;
            return;
        }
        if byte == b'%' && !self.decoding {
            self.decoding = true;
            return;
        }
        if self.decoding && self.encoded.len() == 1 {
            self.encoded.push(byte);
            return self.decode();
        }
        if self.decoding {
            self.encoded.push(byte); 
        }
        if self.parsing_key {
            self.key.push(byte);
        } else {
            self.val.push(byte);
        }
    }

    fn decode(&mut self) {
        let digits = String::from_utf8(self.encoded.clone()).unwrap();
        let number = digits.parse::<u8>().unwrap();
        
        if self.parsing_key {
            self.key.push(number);
        } else {
            self.val.push(number);
        }
    }

    fn complete(&mut self) -> Result<(String, String), ParseErr> {
        if self.decoding {
            self.decode();
        }
        let key = String::from_utf8(self.key.clone()).unwrap();
        let val = String::from_utf8(self.val.clone()).unwrap();

        mem::swap(self, &mut Self::new());

        Ok((key,val)) 
    }
}


fn decode_uri_encoding(data: &[u8]) -> Result<Vec<u8>, ParseErr> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < data.len() {
        if data[i] == b'%' {
            i += 1;

            let start = i;
            while data[i].is_ascii_digit() && i - start < 3 && i < data.len() {
                i += 1;
            }

            let digits = match String::from_utf8(data[start..i].to_vec()) {
                Ok(d) => d,
                Err(inner) => return Err(ParseErr::Encoding { while_parsing: "uri component", inner }),
            };
            let byte = match digits.parse::<u8>() {
                Ok(b) => b,
                Err(_) => return Err(ParseErr::InvalidUriComponent { string: String::from_utf8_lossy_owned(data.to_vec()), index: start }),
            };

            result.push(byte);
        } else {
            i += 1;
        }
    }

    Ok(result)
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
                host: Some("google.com".to_owned()),
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
                host: Some("example.com".to_owned()),
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
                host: Some("example.com".to_owned()),
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
                host: Some("localhost".to_owned()),
                port: Some(1234),
                path: None,
                query: vec![]
            })
        );
    }

    #[test]
    fn test_magnet_uri() {
        assert_eq!(
            Uri::parse("magnet:?xt=urn:btih:8C9E40087DF68D28C233FDFF191D76411B644BEE&dn=The%20Bear%20S04E01%201080p%20WEB%20h264-ETHEL&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Ftracker.bittor.pw%3A1337%2Fannounce&tr=udp%3A%2F%2Fpublic.popcorn-tracker.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.dler.org%3A6969%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969&tr=udp%3A%2F%2Fopen.demonii.com%3A1337%2Fannounce&tr=udp%3A%2F%2Fglotorrents.pw%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969&tr=udp%3A%2F%2Ftorrent.gresille.org%3A80%2Fannounce&tr=udp%3A%2F%2Fp4p.arenabg.com%3A1337&tr=udp%3A%2F%2Ftracker.internetwarriors.net%3A1337"),
            Ok(Uri {
                scheme: "http".to_owned(),
                host: Some("localhost".to_owned()),
                port: Some(1234),
                path: None,
                query: vec![]
            })
        );
    }
}
