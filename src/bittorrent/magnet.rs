use crate::bittorrent::Uri;
use hex::decode;

pub struct Magnet {
    pub info_hash: [u8; 20],
    pub display_name: Option<String>,
    pub trackers: Vec<String>, 
}

pub enum ParseErr {
    InvalidXt { message: &'static str }
}

impl Magnet {
    pub fn parse(uri: Uri) -> Result<Self, ParseErr> {
        let display_name = uri.query("dn").get(0).map(|dn| dn.clone());
        let trackers = uri.query("tr").iter().cloned().collect();

        let mut xt = match uri.query("xt").first().map(|xt| xt.clone()) {
            Some(xt) => xt,
            None => unimplemented!()
        };

        if !xt.starts_with("urn:btih:") {
            return Err(ParseErr::InvalidXt { message: "urn must start with 'urn:btih:'" });
        }

        xt.replace_range(0.."urn:btih".len(), "");

        let data = match decode(xt) {
            Ok(d) => d,
            Err(err) => unimplemented!()
        };

        if data.len() != 20 {
            return Err(ParseErr::InvalidXt { message: "invalid length for info hash" });
        }
        let mut info_hash = [0u8; 20];
        data.iter().enumerate().for_each(|(i, b)| info_hash[i] = *b);


        Ok(Magnet { info_hash, display_name, trackers })
    }
}
