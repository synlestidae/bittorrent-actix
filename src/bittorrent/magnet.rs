use crate::bittorrent::Uri;
use hex::decode;
use log::trace;

#[derive(Clone, Debug)]
pub struct Magnet {
    pub info_hash: [u8; 20],
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
    pub exact_length: Option<usize>,
}

#[derive(Debug)]
pub enum ParseErr {
    InvalidXt { message: &'static str },
}

impl Magnet {
    pub fn parse(uri: Uri) -> Result<Self, ParseErr> {
        trace!("Parsing {:?}", uri);

        let display_name = uri.query("dn").get(0).map(|dn| dn.clone());
        let trackers = uri.query("tr").iter().cloned().collect();

        trace!("display_name: {:?}", display_name);
        trace!("trackers: {:?}", trackers);

        let mut exact_topic = match uri.query("xt").first().map(|xt| xt.clone()) {
            Some(xt) => xt,
            None => unimplemented!(),
        };

        if !exact_topic.starts_with("urn:btih:") {
            return Err(ParseErr::InvalidXt {
                message: "urn must start with 'urn:btih:'",
            });
        }

        exact_topic.replace_range(0.."urn:btih:".len(), "");

        trace!("parsing file hash: {}", exact_topic);

        let data = match decode(exact_topic) {
            Ok(d) => d,
            Err(err) => unimplemented!("error decoding: {}", err),
        };

        if data.len() != 20 {
            return Err(ParseErr::InvalidXt {
                message: "invalid length for info hash",
            });
        }
        let mut info_hash = [0u8; 20];
        data.iter().enumerate().for_each(|(i, b)| info_hash[i] = *b);

        let exact_length = match uri.query("xl").first() {
            Some(xl) => Some(xl.parse::<usize>().unwrap()),
            None => None,
        };

        Ok(Magnet {
            info_hash,
            display_name,
            trackers,
            exact_length,
        })
    }
}
