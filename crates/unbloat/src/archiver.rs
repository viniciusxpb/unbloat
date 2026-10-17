use crate::i18n;
use crate::{
    Result, check_cancel,
    config::Config,
    error,
    manifest::{ArchiveItem, Manifest, check_source},
    output::{self, TemporaryOutput},
};
use sevenz_rust2::{
    ArchiveEntry, ArchiveReader, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password,
    SourceReader, encoder_options::Lzma2Options,
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    path::PathBuf,
    sync::atomic::AtomicBool,
};

/// Opens each source only when the encoder needs it, and releases it at EOF.
/// Thus thousands of small solid entries don't consume thousands of handles.
struct CheckedReader<'a> {
    item: &'a ArchiveItem,
    file: Option<File>,
    read: u64,
    finished: bool,
    cancel: &'a AtomicBool,
}
impl<'a> CheckedReader<'a> {
    fn new(item: &'a ArchiveItem, cancel: &'a AtomicBool) -> Self {
        Self {
            item,
            cancel,
            file: None,
            read: 0,
            finished: false,
        }
    }
    fn read_checked(&mut self, buf: &mut [u8]) -> Result<usize> {
        check_cancel(self.cancel)?;
        if self.finished || buf.is_empty() {
            return Ok(0);
        }
        if self.file.is_none() {
            check_source(self.item)?;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.share_mode(1).custom_flags(0x00200000); // Deny writes/deletes; open reparse points themselves.
            }
            let file = options.open(&self.item.source_path)?;
            let meta = file.metadata()?;
            if crate::manifest::is_link(&meta)
                || !meta.is_file()
                || meta.len() != self.item.size
                || meta.modified()? != self.item.modified
            {
                return Err(error(i18n::text("archive.changed_before_read")));
            }
            self.file = Some(file);
        }
        let count = self.file.as_mut().unwrap().read(buf)?;
        self.read += count as u64;
        if count == 0 {
            if self.read != self.item.size {
                return Err(error(i18n::text("archive.size_changed")));
            }
            check_source(self.item)?;
            self.file = None;
            self.finished = true;
        }
        Ok(count)
    }
}
impl Read for CheckedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_checked(buf)
            .map_err(|e| io::Error::other(format!("{}: {e}", self.item.source_path.display())))
    }
}

pub fn create(
    manifest: &Manifest,
    config: &Config,
    cancel: &AtomicBool,
    progress: &dyn Fn(&str),
) -> Result<PathBuf> {
    config.validate()?;
    manifest.validate()?;
    check_cancel(cancel)?;
    let (temporary, file) = TemporaryOutput::create(&manifest.destination)?;
    let mut writer = ArchiveWriter::new(file)?;
    writer.set_content_methods(vec![
        EncoderConfiguration::new(EncoderMethod::LZMA2)
            .with_options(Lzma2Options::from_level(config.compression.level).into()),
    ]);
    let cap = config.compression.solid_block_size_mb * 1024 * 1024;
    let mut block: Vec<&ArchiveItem> = Vec::new();
    let mut bytes = 0u64;
    let flush = |writer: &mut ArchiveWriter<File>, block: &mut Vec<&ArchiveItem>| -> Result<()> {
        if !block.is_empty() {
            let entries = block
                .iter()
                .map(|item| ArchiveEntry::new_file(&item.archive_path))
                .collect();
            let readers = block
                .iter()
                .map(|item| SourceReader::new(CheckedReader::new(item, cancel)))
                .collect();
            writer.push_archive_entries(entries, readers)?;
            block.clear();
        }
        Ok(())
    };
    for item in &manifest.items {
        check_cancel(cancel)?;
        progress(&item.archive_path);
        if item.is_directory || item.size == 0 {
            check_source(item)?;
            // An empty file still gets opened to surface sharing/permission failures.
            if !item.is_directory {
                let mut r = CheckedReader::new(item, cancel);
                if r.read(&mut [0u8; 1])? != 0 {
                    return Err(error(i18n::text("archive.empty_changed")));
                }
            }
            let entry = if item.is_directory {
                ArchiveEntry::new_directory(&item.archive_path)
            } else {
                ArchiveEntry::new_file(&item.archive_path)
            };
            writer.push_archive_entry::<File>(entry, None)?;
        } else if config.compression.solid && item.size <= cap {
            if bytes + item.size > cap || block.len() >= 4096 {
                flush(&mut writer, &mut block)?;
                bytes = 0;
            }
            block.push(item);
            bytes += item.size;
        } else {
            flush(&mut writer, &mut block)?;
            bytes = 0;
            writer.push_archive_entry(
                ArchiveEntry::new_file(&item.archive_path),
                Some(CheckedReader::new(item, cancel)),
            )?;
        }
    }
    flush(&mut writer, &mut block)?;
    let file = writer.finish()?;
    file.sync_all()?;
    drop(file);
    progress(i18n::text("progress.verifying"));
    // Read every stream back, validating decompression and CRC before publication.
    let mut reader = ArchiveReader::open(&temporary.path, Password::empty())?;
    if reader.archive().files.len() != manifest.items.len() {
        return Err(error(i18n::text("archive.entry_count")));
    }
    reader.for_each_entries(|_, stream| {
        let mut buf = [0u8; 65536];
        loop {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(io::Error::other(i18n::text("engine.cancelled_short")).into());
            }
            if stream.read(&mut buf)? == 0 {
                break;
            }
        }
        Ok(true)
    })?;
    drop(reader);
    // Detect modifications to sources already read, before publishing the result.
    for item in &manifest.items {
        check_cancel(cancel)?;
        check_source(item)?;
    }
    temporary.publish(
        &manifest.destination,
        &output::stem(manifest, &config.output)?,
    )
}
