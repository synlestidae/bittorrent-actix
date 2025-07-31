#![feature(bufreader_peek)]
use actix::prelude::*;
use log::trace;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::exit;

const MAX_BYTE_STRING_LEN: i64 = 1 << 32;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Bencoding {
    Integer(i64),
    ByteString(Vec<u8>),
    List(Vec<Bencoding>),
    Dictionary(HashMap<Vec<u8>, Bencoding>),
}

impl Bencoding {
    pub fn serialize<W: Write>(element: &Bencoding, output: &mut W) {
        unimplemented!()
    }

    pub fn deserialize<R: Read>(reader: &mut BufReader<R>) -> Result<Bencoding, ()> {
        match peek_char(reader) {
            Some('i') => {
                trace!("Deserialising integer");

                eat_char(reader, 'i')?;

                let num = read_decimal(reader);

                eat_char(reader, 'e')?;

                trace!("Deserialised integer={}", num);
                return Ok(Bencoding::Integer(num));
            }
            Some('l') => {
                trace!("Deserialising list");

                eat_char(reader, 'l')?;

                let mut list = Vec::new();
                while peek_char(reader) != Some('e') {
                    let element = Bencoding::deserialize(reader)?;
                    list.push(element);
                }

                eat_char(reader, 'e')?;

                trace!("Deserialised list of length {}", list.len());
                return Ok(Bencoding::List(list));
            }
            Some('d') => {
                trace!("Deserialising dictionary");

                eat_char(reader, 'd')?;

                let mut dict = HashMap::new();

                while peek_char(reader) != Some('e') {
                    let key = read_string(reader)?;
                    let value = Bencoding::deserialize(reader)?;

                    dict.insert(key, value);
                }

                eat_char(reader, 'e')?;

                trace!("Deserialised dictionary");
                Ok(Bencoding::Dictionary(dict))
            }
            Some(c) if c.is_ascii_digit() => {
                trace!("Deserialising byte string");

                let string = read_string(reader)?;

                trace!("Deserialised byte string of length {}", string.len());
                return Ok(Bencoding::ByteString(string));
            }
            _ => unimplemented!(),
        }
    }

    pub fn is_list(&self) -> bool {
        match self {
            &Bencoding::List(_) => true,
            _ => false,
        }
    }
}

fn read_string<R: Read>(reader: &mut BufReader<R>) -> Result<Vec<u8>, ()> {
    let length = read_decimal(reader);

    eat_char(reader, ':')?;

    if length > MAX_BYTE_STRING_LEN {
        unimplemented!("Declared byte length too long");
    }
    let mut byte_buffer = vec![0; length as usize];
    reader.read_exact(&mut byte_buffer).unwrap();

    return Ok(byte_buffer);
}

fn read_decimal<R: Read>(reader: &mut BufReader<R>) -> i64 {
    let mut digit_vec = Vec::new();
    while peek_char(reader)
        .expect("Unexpected end of stream while reading digits")
        .is_ascii_digit()
        || peek_char(reader) == Some('-')
    {
        let c = read_char(reader);
        if let Some(digit) = c {
            digit_vec.push(digit as u8);
        } else {
            unimplemented!("Unexpected end of stream while reading digits");
        }
    }

    let byte_length = String::from_utf8_lossy(&digit_vec).parse::<i64>().unwrap();
    byte_length
}

fn eat_char<R: Read>(reader: &mut BufReader<R>, expected_char: char) -> Result<(), ()> {
    match read_char(reader) {
        Some(actual_char) => {
            if expected_char != actual_char {
                return Err(());
            }

            Ok(())
        }
        None => Err(()),
    }
}

fn read_char<R: Read>(reader: &mut BufReader<R>) -> Option<char> {
    let mut c: Vec<u8> = vec![0; 1];
    match reader.read_exact(&mut c) {
        Ok(_) => Some(c[0] as char),
        _ => None,
    }
}

fn peek_char<R: Read>(reader: &mut BufReader<R>) -> Option<char> {
    match reader.peek(1) {
        Ok(&[c]) => Some(c as char),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use crate::bencoding::Bencoding;
    use std::io::{BufReader, Cursor, Seek};

    #[test]
    fn test_deserialize_byte_string_spam() {
        let mut cursor = BufReader::new(Cursor::new("4:spam".as_bytes().to_vec()));

        assert_eq!(
            Bencoding::deserialize(&mut cursor),
            Ok(Bencoding::ByteString("spam".as_bytes().to_vec()))
        );
        assert_eq!(
            cursor.stream_position().unwrap(),
            cursor.get_ref().get_ref().len() as u64
        );
    }

    #[test]
    fn test_deserialize_integer_3() {
        let mut cursor = BufReader::new(Cursor::new("i3e".as_bytes().to_vec()));

        assert_eq!(
            Bencoding::deserialize(&mut cursor),
            Ok(Bencoding::Integer(3))
        );
        assert_eq!(
            cursor.stream_position().unwrap(),
            cursor.get_ref().get_ref().len() as u64
        );
    }

    #[test]
    fn test_deserialize_integer_negative_3() {
        let mut cursor = BufReader::new(Cursor::new("i-3e".as_bytes().to_vec()));

        assert_eq!(
            Bencoding::deserialize(&mut cursor),
            Ok(Bencoding::Integer(-3))
        );
        assert_eq!(
            cursor.stream_position().unwrap(),
            cursor.get_ref().get_ref().len() as u64
        );
    }

    #[test]
    fn test_deserialize_list_empty() {
        let mut cursor = BufReader::new(Cursor::new("le".as_bytes().to_vec()));

        assert_eq!(
            Bencoding::deserialize(&mut cursor),
            Ok(Bencoding::List(Vec::new()))
        );
        assert_eq!(
            cursor.stream_position().unwrap(),
            cursor.get_ref().get_ref().len() as u64
        );
    }

    #[test]
    fn test_deserialize_list_spam_eggs() {
        let mut cursor = BufReader::new(Cursor::new("l4:spam4:eggse".as_bytes().to_vec()));

        assert_eq!(
            Bencoding::deserialize(&mut cursor),
            Ok(Bencoding::List(vec![
                Bencoding::ByteString("spam".as_bytes().to_vec()),
                Bencoding::ByteString("eggs".as_bytes().to_vec())
            ]))
        );
        assert_eq!(
            cursor.stream_position().unwrap(),
            cursor.get_ref().get_ref().len() as u64
        );
    }
}
