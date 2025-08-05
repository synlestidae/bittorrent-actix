#![feature(bufreader_peek)]
use log::trace;
use std::collections::HashMap;
use std::io::{BufReader, Error as IOErr, Read, Seek, Write};

const MAX_BYTE_STRING_LEN: i64 = 1 << 32;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Position {
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Bencoding {
    Integer {
        integer: i64,
        position: Option<Position>,
    },
    ByteString {
        byte_string: Vec<u8>,
        position: Option<Position>,
    },
    List {
        list: Vec<Bencoding>,
        position: Option<Position>,
    },
    Dictionary {
        dictionary: HashMap<Vec<u8>, Bencoding>,
        position: Option<Position>,
    },
}

impl Bencoding {
    pub fn serialize<W: Write>(&self, output: &mut W) -> Result<(), IOErr> {
        match self {
            Bencoding::Integer { integer, .. } => {
                output.write_all("i".as_bytes())?;
                output.write_all(format!("{}", integer).as_bytes())?;
                output.write_all("e".as_bytes())?;
            }
            Bencoding::ByteString { byte_string, .. } => {
                output.write_all(format!("{}", byte_string.len()).as_bytes())?;
                output.write_all(":".as_bytes())?;
                output.write_all(byte_string)?;
            }
            Bencoding::List { list, .. } => {
                output.write_all("l".as_bytes())?;
                for element in list.iter() {
                    element.serialize(output)?;
                }
                output.write_all("e".as_bytes())?;
            }
            Bencoding::Dictionary { dictionary, .. } => {
                output.write_all("d".as_bytes())?;
                for (key, value) in dictionary {
                    Bencoding::ByteString {
                        byte_string: key.clone(),
                        position: None,
                    }
                    .serialize(output)?;
                    value.serialize(output)?;
                }
                output.write_all("e".as_bytes())?;
            }
        }

        Ok(())
    }

    pub fn deserialize<R: Read + Seek>(reader: &mut BufReader<R>) -> Result<Bencoding, ()> {
        let offset = reader.stream_position().unwrap(); // TODO

        match peek_char(reader) {
            Some('i') => {
                trace!("Deserialising integer");

                eat_char(reader, 'i')?;

                let integer = read_decimal(reader);

                eat_char(reader, 'e')?;

                trace!("Deserialised integer={}", integer);

                let length = reader.stream_position().unwrap() - offset;

                Ok(Bencoding::Integer {
                    integer,
                    position: Some(Position { offset, length }),
                })
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

                let length = reader.stream_position().unwrap() - offset;

                trace!("Deserialised list of length {}", list.len());

                Ok(Bencoding::List {
                    list,
                    position: Some(Position { offset, length }),
                })
            }
            Some('d') => {
                trace!("Deserialising dictionary");

                eat_char(reader, 'd')?;

                let mut dictionary = HashMap::new();

                while peek_char(reader) != Some('e') {
                    let key = read_string(reader)?;
                    let value = Bencoding::deserialize(reader)?;

                    dictionary.insert(key, value);
                }

                eat_char(reader, 'e')?;

                let length = reader.stream_position().unwrap() - offset;

                trace!("Deserialised dictionary");

                Ok(Bencoding::Dictionary {
                    dictionary,
                    position: Some(Position { offset, length }),
                })
            }
            Some(c) if c.is_ascii_digit() => {
                trace!("Deserialising byte string");

                let byte_string = read_string(reader)?;

                trace!("Deserialised byte string of length {}", byte_string.len());

                let length = reader.stream_position().unwrap() - offset;

                Ok(Bencoding::ByteString {
                    byte_string,
                    position: Some(Position { offset, length }),
                })
            }
            _ => unimplemented!(),
        }
    }

    pub fn is_list(&self) -> bool {
        match self {
            &Bencoding::List { .. } => true,
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

    Ok(byte_buffer)
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

    String::from_utf8_lossy(&digit_vec).parse::<i64>().unwrap()
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
            Ok(Bencoding::ByteString {
                byte_string: "spam".as_bytes().to_vec()
            })
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
            Ok(Bencoding::Integer { integer: 3 })
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
            Ok(Bencoding::Integer { integer: -3 })
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
            Ok(Bencoding::List { list: Vec::new() })
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
