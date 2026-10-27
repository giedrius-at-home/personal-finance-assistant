//! Read float32 tensors from a `.safetensors` file one at a time, without
//! memory-mapping it: an 8-byte little-endian header length, a JSON header
//! naming each tensor's dtype, shape and byte range, then the raw data.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::Error;

/// Headers larger than this are not a TimesFM checkpoint.
const MAX_HEADER: u64 = 16 << 20;

#[derive(Debug)]
struct Entry {
    shape: Vec<usize>,
    start: u64,
    end: u64,
}

/// An open checkpoint: the header is parsed, tensors are read on demand.
#[derive(Debug)]
pub(crate) struct Checkpoint {
    file: BufReader<File>,
    data_start: u64,
    entries: HashMap<String, Entry>,
}

impl Checkpoint {
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let mut file = BufReader::new(File::open(path)?);
        let file_len = file.get_ref().metadata()?.len();
        let mut len = [0u8; 8];
        file.read_exact(&mut len)?;
        let header_len = u64::from_le_bytes(len);
        if header_len > MAX_HEADER || header_len + 8 > file_len {
            return Err(Error::Checkpoint("header length out of range".into()));
        }
        let mut header = vec![0u8; usize::try_from(header_len).unwrap_or(0)];
        file.read_exact(&mut header)?;
        let header: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&header)
            .map_err(|e| Error::Checkpoint(format!("header: {e}")))?;
        let data_start = 8 + header_len;
        let mut entries = HashMap::new();
        for (name, info) in header {
            if name == "__metadata__" {
                continue;
            }
            let dtype = info.get("dtype").and_then(|d| d.as_str());
            if dtype != Some("F32") {
                return Err(Error::Checkpoint(format!("{name}: expected F32")));
            }
            let numbers = |key: &str| -> Option<Vec<u64>> {
                info.get(key)?
                    .as_array()?
                    .iter()
                    .map(serde_json::Value::as_u64)
                    .collect()
            };
            let (Some(shape), Some(offsets)) = (numbers("shape"), numbers("data_offsets")) else {
                return Err(Error::Checkpoint(format!("{name}: malformed entry")));
            };
            let [start, end] = offsets[..] else {
                return Err(Error::Checkpoint(format!("{name}: malformed offsets")));
            };
            let shape: Vec<usize> = shape
                .into_iter()
                .map(|d| usize::try_from(d).unwrap_or(usize::MAX))
                .collect();
            let count: u64 = shape.iter().map(|d| *d as u64).product();
            if end < start || end - start != count * 4 || data_start + end > file_len {
                return Err(Error::Checkpoint(format!("{name}: data out of range")));
            }
            entries.insert(name, Entry { shape, start, end });
        }
        Ok(Self {
            file,
            data_start,
            entries,
        })
    }

    /// The named tensor's values (row-major) after checking its shape.
    pub(crate) fn read(&mut self, name: &str, shape: &[usize]) -> Result<Vec<f32>, Error> {
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| Error::Checkpoint(format!("missing tensor {name}")))?;
        if entry.shape != shape {
            return Err(Error::Checkpoint(format!(
                "{name}: shape {:?}, expected {shape:?}",
                entry.shape
            )));
        }
        let mut bytes = vec![0u8; usize::try_from(entry.end - entry.start).unwrap_or(0)];
        self.file
            .seek(SeekFrom::Start(self.data_start + entry.start))?;
        self.file.read_exact(&mut bytes)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, header: &str, data: &[f32]) {
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header.as_bytes());
        for v in data {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn reads_tensors_by_name_and_checks_shapes() {
        let dir = std::env::temp_dir().join(format!("timesfm-st-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.safetensors");
        let header = r#"{"__metadata__":{"format":"pt"},"a":{"dtype":"F32","shape":[2,2],"data_offsets":[0,16]},"b":{"dtype":"F32","shape":[1],"data_offsets":[16,20]}}"#;
        write(&path, header, &[1.0, 2.0, 3.0, 4.0, -0.5]);
        let mut checkpoint = Checkpoint::open(&path).unwrap();
        assert_eq!(checkpoint.read("b", &[1]).unwrap(), [-0.5]);
        assert_eq!(checkpoint.read("a", &[2, 2]).unwrap(), [1.0, 2.0, 3.0, 4.0]);
        assert!(checkpoint.read("a", &[4]).is_err());
        assert!(checkpoint.read("c", &[1]).is_err());

        let bad = r#"{"a":{"dtype":"F32","shape":[4],"data_offsets":[0,16]}}"#;
        write(&path, bad, &[1.0]);
        assert!(Checkpoint::open(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
