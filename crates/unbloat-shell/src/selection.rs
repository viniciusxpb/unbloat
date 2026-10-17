//! Small shared transport protocol. No dependency on the archive engine.
#![cfg(windows)]
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8; 8] = b"UNBLOAT2";
const MAX_BYTES: u64 = 32 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn write(paths: &[PathBuf]) -> io::Result<PathBuf> {
    if paths.is_empty() || paths.len() > 10000 {
        return Err(io::Error::other("Seleção vazia ou grande demais."));
    }
    let mut data = Vec::new();
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&(paths.len() as u32).to_le_bytes());
    for path in paths {
        let units: Vec<u16> = path.as_os_str().encode_wide().collect();
        if !path.is_absolute() || units.len() > 32767 || units.contains(&0) {
            return Err(io::Error::other("Caminho selecionado inválido."));
        }
        data.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for unit in units {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        if data.len() as u64 > MAX_BYTES {
            return Err(io::Error::other("Seleção grande demais."));
        }
    }
    loop {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "unbloat-selection-{}-{nanos}-{}.bin",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let result = file.write_all(&data).and_then(|_| file.sync_all());
                drop(file);
                if let Err(e) = result {
                    let _ = fs::remove_file(&path);
                    return Err(e);
                }
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

pub fn read_and_remove(path: &Path) -> io::Result<Vec<PathBuf>> {
    use std::os::windows::fs::MetadataExt;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Transporte sem diretório."))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !name.starts_with("unbloat-selection-")
        || !name.ends_with(".bin")
        || parent.canonicalize()? != std::env::temp_dir().canonicalize()?
    {
        return Err(io::Error::other(
            "Arquivo de seleção fora do transporte temporário do Unbloat.",
        ));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return Err(io::Error::other("Transporte não é um arquivo regular."));
    }
    let result = (|| {
        let mut file = OpenOptions::new().read(true).share_mode(1).open(path)?;
        if file.metadata()?.len() > MAX_BYTES {
            return Err(io::Error::other("Transporte grande demais."));
        }
        let mut data = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_BYTES + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 > MAX_BYTES {
            return Err(io::Error::other("Transporte grande demais."));
        }
        let mut cursor = io::Cursor::new(data);
        let mut magic = [0u8; 8];
        cursor.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(io::Error::other("Transporte de seleção inválido."));
        }
        fn number(r: &mut impl Read) -> io::Result<u32> {
            let mut b = [0; 4];
            r.read_exact(&mut b)?;
            Ok(u32::from_le_bytes(b))
        }
        let count = number(&mut cursor)?;
        if count == 0 || count > 10000 {
            return Err(io::Error::other("Quantidade de pastas inválida."));
        }
        let mut paths = Vec::new();
        for _ in 0..count {
            let len = number(&mut cursor)?;
            if len == 0 || len > 32767 {
                return Err(io::Error::other("Comprimento de caminho inválido."));
            }
            let mut units = Vec::with_capacity(len as usize);
            for _ in 0..len {
                let mut b = [0; 2];
                cursor.read_exact(&mut b)?;
                units.push(u16::from_le_bytes(b));
            }
            if units.contains(&0) {
                return Err(io::Error::other("NUL no caminho."));
            }
            let path = PathBuf::from(OsString::from_wide(&units));
            if !path.is_absolute() {
                return Err(io::Error::other("Seleção deve conter caminhos absolutos."));
            }
            paths.push(path);
        }
        if cursor.position() != cursor.get_ref().len() as u64 {
            return Err(io::Error::other("Dados extras no transporte."));
        }
        Ok(paths)
    })();
    let removal = fs::remove_file(path);
    let paths = result?;
    removal?;
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_cleanup() {
        let paths = vec![
            PathBuf::from("C:\\projetos\\olá mundo"),
            PathBuf::from("C:\\projetos\\backend"),
        ];
        let path = write(&paths).unwrap();
        assert_eq!(read_and_remove(&path).unwrap(), paths);
        assert!(!path.exists());
        let path = write(&paths).unwrap();
        fs::write(&path, b"invalid").unwrap();
        assert!(read_and_remove(&path).is_err());
        assert!(!path.exists());
    }
}
