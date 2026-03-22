use bendy::decoding::{Decoder, DictDecoder, Object};
use crate::{
    error::{ProtocolError, Result},
    info_hash::{InfoHash, InfoHashV1, InfoHashV2},
};

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: Vec<String>,
    pub length: u64,
    pub pieces_root: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
pub struct Info {
    pub name: String,
    pub piece_length: u64,
    pub pieces: Option<Vec<u8>>,
    pub meta_version: Option<u8>,
    pub length: Option<u64>,
    pub files: Option<Vec<FileInfo>>,
    pub raw_bytes: Vec<u8>,
}

impl Info {
    pub fn info_hash_v1(&self) -> Option<InfoHashV1> {
        self.pieces.as_ref().map(|_| InfoHashV1::from_info_dict(&self.raw_bytes))
    }
    pub fn info_hash_v2(&self) -> Option<InfoHashV2> {
        (self.meta_version == Some(2)).then(|| InfoHashV2::from_info_dict(&self.raw_bytes))
    }
    pub fn info_hash(&self) -> Option<InfoHash> {
        match (self.info_hash_v1(), self.info_hash_v2()) {
            (Some(v1), Some(v2)) => Some(InfoHash::Hybrid { v1, v2 }),
            (Some(v1), None)     => Some(InfoHash::V1(v1)),
            (None, Some(v2))     => Some(InfoHash::V2(v2)),
            (None, None)         => None,
        }
    }
    pub fn total_length(&self) -> u64 {
        self.length.unwrap_or_else(|| {
            self.files.as_deref().unwrap_or(&[]).iter().map(|f| f.length).sum()
        })
    }
}

#[derive(Debug, Clone)]
pub struct Metainfo {
    pub info: Info,
    pub announce: Option<String>,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub creation_date: Option<i64>,
}

impl Metainfo {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(data);
        let obj = decoder.next_object()
            .map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))?
            .ok_or_else(|| ProtocolError::BencodeDecodeError("empty input".into()))?;

        let mut dict = match obj {
            Object::Dict(d) => d,
            _ => return Err(ProtocolError::BencodeDecodeError("top-level must be a dict".into())),
        };

        let mut announce = None;
        let mut announce_list = None;
        let mut comment = None;
        let mut created_by = None;
        let mut creation_date = None;
        let mut info_raw: Option<Vec<u8>> = None;

        while let Some((key, value)) = dict.next_pair()
            .map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))?
        {
            match key {
                b"announce"      => { if let Object::Bytes(b) = value { announce = Some(String::from_utf8_lossy(b).into_owned()); } }
                b"comment"       => { if let Object::Bytes(b) = value { comment = Some(String::from_utf8_lossy(b).into_owned()); } }
                b"created by"    => { if let Object::Bytes(b) = value { created_by = Some(String::from_utf8_lossy(b).into_owned()); } }
                b"creation date" => { if let Object::Integer(i) = value { creation_date = i.parse::<i64>().ok(); } }
                b"announce-list" => { announce_list = Some(parse_announce_list(value)?); }
                b"info"          => { if let Object::Dict(d) = value { info_raw = Some(encode_dict_raw(d)?); } }
                _ => { let _ = value; }
            }
        }

        let info_bytes = info_raw.ok_or(ProtocolError::MissingField("info"))?;
        let info = parse_info(&info_bytes)?;
        Ok(Metainfo { info, announce, announce_list, comment, created_by, creation_date })
    }
}

fn parse_announce_list(obj: Object<'_, '_>) -> Result<Vec<Vec<String>>> {
    let mut outer = match obj { Object::List(l) => l, _ => return Ok(vec![]) };
    let mut result = Vec::new();
    while let Some(tier_obj) = outer.next_object().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
        let mut tier_list = match tier_obj { Object::List(l) => l, _ => continue };
        let mut tier = Vec::new();
        while let Some(url_obj) = tier_list.next_object().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
            if let Object::Bytes(b) = url_obj { tier.push(String::from_utf8_lossy(b).into_owned()); }
        }
        result.push(tier);
    }
    Ok(result)
}

fn encode_object(obj: Object<'_, '_>) -> Result<Vec<u8>> {
    match obj {
        Object::Bytes(b) => { let mut o = format!("{}:", b.len()).into_bytes(); o.extend_from_slice(b); Ok(o) }
        Object::Integer(i) => Ok(format!("i{}e", i).into_bytes()),
        Object::List(mut list) => {
            let mut o = vec![b'l'];
            while let Some(item) = list.next_object().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
                o.extend_from_slice(&encode_object(item)?);
            }
            o.push(b'e');
            Ok(o)
        }
        Object::Dict(mut dict) => {
            let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
            while let Some((k, v)) = dict.next_pair().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
                pairs.push((k.to_vec(), encode_object(v)?));
            }
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            let mut o = vec![b'd'];
            for (k, v) in pairs { o.extend_from_slice(format!("{}:", k.len()).as_bytes()); o.extend_from_slice(&k); o.extend_from_slice(&v); }
            o.push(b'e');
            Ok(o)
        }
    }
}

fn encode_dict_raw(mut dict: DictDecoder<'_, '_>) -> Result<Vec<u8>> {
    let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    while let Some((k, v)) = dict.next_pair().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
        pairs.push((k.to_vec(), encode_object(v)?));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = vec![b'd'];
    for (k, v) in pairs { out.extend_from_slice(format!("{}:", k.len()).as_bytes()); out.extend_from_slice(&k); out.extend_from_slice(&v); }
    out.push(b'e');
    Ok(out)
}

fn parse_info(raw: &[u8]) -> Result<Info> {
    let mut decoder = Decoder::new(raw);
    let obj = decoder.next_object().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))?
        .ok_or_else(|| ProtocolError::BencodeDecodeError("empty info dict".into()))?;
    let mut dict = match obj { Object::Dict(d) => d, _ => return Err(ProtocolError::BencodeDecodeError("info must be a dict".into())) };

    let mut name = None; let mut piece_length = None; let mut pieces = None;
    let mut meta_version = None; let mut length = None; let mut files = None;

    while let Some((key, value)) = dict.next_pair().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
        match key {
            b"name"         => { if let Object::Bytes(b) = value { name = Some(String::from_utf8_lossy(b).into_owned()); } }
            b"piece length" => { if let Object::Integer(i) = value { piece_length = i.parse::<u64>().ok(); } }
            b"pieces"       => { if let Object::Bytes(b) = value { pieces = Some(b.to_vec()); } }
            b"meta version" => { if let Object::Integer(i) = value { meta_version = i.parse::<u8>().ok(); } }
            b"length"       => { if let Object::Integer(i) = value { length = i.parse::<u64>().ok(); } }
            b"files"        => { files = Some(parse_files(value)?); }
            _ => { let _ = value; }
        }
    }

    Ok(Info {
        name: name.ok_or(ProtocolError::MissingField("info.name"))?,
        piece_length: piece_length.ok_or(ProtocolError::MissingField("info.piece length"))?,
        pieces, meta_version, length, files,
        raw_bytes: raw.to_vec(),
    })
}

fn parse_files(obj: Object<'_, '_>) -> Result<Vec<FileInfo>> {
    let mut list = match obj { Object::List(l) => l, _ => return Err(ProtocolError::BencodeDecodeError("files must be a list".into())) };
    let mut files = Vec::new();
    while let Some(item) = list.next_object().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
        let mut dict = match item { Object::Dict(d) => d, _ => continue };
        let mut path = None; let mut file_length = None; let mut pieces_root = None;
        while let Some((k, v)) = dict.next_pair().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
            match k {
                b"path"        => { path = Some(parse_path_list(v)?); }
                b"length"      => { if let Object::Integer(i) = v { file_length = i.parse::<u64>().ok(); } }
                b"pieces root" => { if let Object::Bytes(b) = v { if b.len() == 32 { let mut arr = [0u8; 32]; arr.copy_from_slice(b); pieces_root = Some(arr); } } }
                _ => { let _ = v; }
            }
        }
        files.push(FileInfo { path: path.unwrap_or_default(), length: file_length.unwrap_or(0), pieces_root });
    }
    Ok(files)
}

fn parse_path_list(obj: Object<'_, '_>) -> Result<Vec<String>> {
    let mut list = match obj { Object::List(l) => l, _ => return Ok(vec![]) };
    let mut parts = Vec::new();
    while let Some(item) = list.next_object().map_err(|e| ProtocolError::BencodeDecodeError(e.to_string()))? {
        if let Object::Bytes(b) = item { parts.push(String::from_utf8_lossy(b).into_owned()); }
    }
    Ok(parts)
}
