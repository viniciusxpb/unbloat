use crate::i18n;
use crate::{
    Result, check_cancel,
    config::Config,
    error,
    manifest::{ArchiveItem, Manifest, check_source},
    output::{self, TemporaryOutput},
    tracker::{Phase, Tracker},
};
use sevenz_rust2::{
    ArchiveEntry, ArchiveReader, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password,
    SourceReader, encoder_options::Lzma2Options,
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
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
    tracker: &'a Tracker,
    progress: &'a dyn Fn(&str),
}
impl<'a> CheckedReader<'a> {
    fn new(
        item: &'a ArchiveItem,
        cancel: &'a AtomicBool,
        tracker: &'a Tracker,
        progress: &'a dyn Fn(&str),
    ) -> Self {
        Self {
            item,
            cancel,
            tracker,
            progress,
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
            // Reported here, not when queued: solid blocks are encoded later, in one go.
            (self.progress)(&self.item.archive_path);
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
        self.tracker.read(count as u64);
        if count == 0 {
            if self.read != self.item.size {
                return Err(error(i18n::text("archive.size_changed")));
            }
            check_source(self.item)?;
            self.file = None;
            self.finished = true;
            self.tracker.file_done();
        }
        Ok(count)
    }
}
impl Read for CheckedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_checked(buf).map_err(|e| {
            io::Error::other(format!(
                "{}: {e}",
                crate::display_path(&self.item.source_path)
            ))
        })
    }
}

/// Records the archive size as the encoder writes it, for progress reporting.
struct CountingWriter<'a> {
    file: File,
    position: u64,
    tracker: &'a Tracker,
}
impl Write for CountingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let count = self.file.write(buf)?;
        self.position += count as u64;
        self.tracker.written_up_to(self.position);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Seek for CountingWriter<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.position = self.file.seek(position)?;
        Ok(self.position)
    }
}

pub fn create(
    manifest: &Manifest,
    config: &Config,
    cancel: &AtomicBool,
    progress: &dyn Fn(&str),
) -> Result<PathBuf> {
    create_tracked(manifest, config, cancel, progress, &Tracker::default())
}

/// Like `create`, also publishing live counters to `tracker`.
pub fn create_tracked(
    manifest: &Manifest,
    config: &Config,
    cancel: &AtomicBool,
    progress: &dyn Fn(&str),
    tracker: &Tracker,
) -> Result<PathBuf> {
    config.validate()?;
    manifest.validate()?;
    check_cancel(cancel)?;
    let files = manifest.items.iter().filter(|item| !item.is_directory);
    tracker.start(
        files.clone().count() as u64,
        files.map(|item| item.size).sum(),
    );
    let (temporary, file) = TemporaryOutput::create(&manifest.destination)?;
    let mut writer = ArchiveWriter::new(CountingWriter {
        file,
        position: 0,
        tracker,
    })?;
    writer.set_content_methods(vec![
        EncoderConfiguration::new(EncoderMethod::LZMA2)
            .with_options(Lzma2Options::from_level(config.compression.level).into()),
    ]);
    let cap = config.compression.solid_block_size_mb * 1024 * 1024;
    let mut block: Vec<&ArchiveItem> = Vec::new();
    let mut bytes = 0u64;
    let flush = |writer: &mut ArchiveWriter<CountingWriter>,
                 block: &mut Vec<&ArchiveItem>|
     -> Result<()> {
        if !block.is_empty() {
            let entries = block
                .iter()
                .map(|item| ArchiveEntry::new_file(&item.archive_path))
                .collect();
            let readers = block
                .iter()
                .map(|item| SourceReader::new(CheckedReader::new(item, cancel, tracker, progress)))
                .collect();
            writer.push_archive_entries(entries, readers)?;
            block.clear();
        }
        Ok(())
    };
    for item in &manifest.items {
        check_cancel(cancel)?;
        if item.is_directory || item.size == 0 {
            check_source(item)?;
            if item.is_directory {
                progress(&item.archive_path);
            } else {
                // An empty file still gets opened to surface sharing/permission failures.
                if CheckedReader::new(item, cancel, tracker, progress).read(&mut [0u8; 1])? != 0 {
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
                Some(CheckedReader::new(item, cancel, tracker, progress)),
            )?;
        }
    }
    flush(&mut writer, &mut block)?;
    let file = writer.finish()?.file;
    file.sync_all()?;
    drop(file);
    tracker.set_phase(Phase::Verifying);
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
            let count = stream.read(&mut buf)?;
            if count == 0 {
                break;
            }
            tracker.verified(count as u64);
        }
        Ok(true)
    })?;
    drop(reader);
    // Detect modifications to sources already read, before publishing the result.
    for item in &manifest.items {
        check_cancel(cancel)?;
        check_source(item)?;
    }
    let published = temporary.publish(
        &manifest.destination,
        &output::stem(manifest, &config.output)?,
    )?;
    tracker.set_phase(Phase::Done);
    Ok(published)
}
