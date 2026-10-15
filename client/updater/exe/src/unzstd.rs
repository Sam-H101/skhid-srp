//! Reading a zstd-compressed file while it is decompressed (`ruzstd`, in
//! pure Rust, no `std`): the database patches' blobs, and a pristine
//! `resources.db` downloaded compressed. One frame after the other, as the
//! `zstd` tool writes them. Its own checksum is not checked: what comes out
//! is checked by SHA-256 by the caller.

use alloc::format;
use alloc::string::String;

use ruzstd::decoding::{FrameDecoder, StreamingDecoder};
use ruzstd::io::{Error, ErrorKind, Read};
use updater_logic::dbpatch::FileId;
use updater_logic::sha256::Sha256;

use crate::win::{self, File};

/// The first bytes of a zstd frame.
pub const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// A file read from its start, which knows how much of it is left.
pub struct Source {
    file: File,
    at: u64,
    size: u64,
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, Error> {
        let read = self
            .file
            .read(buffer)
            .map_err(|_| Error::from(ErrorKind::Other))?;
        self.at += read as u64;
        Ok(read)
    }
}

/// The decompressed bytes of a file.
pub struct Zstd {
    inner: Option<StreamingDecoder<Source, FrameDecoder>>,
}

impl Zstd {
    pub fn open(file: File) -> Result<Self, String> {
        let size = file.size().map_err(|code| win::error_text(code))?;
        file.seek(0).map_err(|code| win::error_text(code))?;
        let source = Source { file, at: 0, size };
        let inner =
            StreamingDecoder::new(source).map_err(|error| format!("not zstd: {error:?}"))?;
        Ok(Self { inner: Some(inner) })
    }

    /// Fills `buffer` as far as the data goes. 0: the end of the last frame.
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        loop {
            let Some(decoder) = self.inner.as_mut() else {
                return Ok(0);
            };
            let read = decoder
                .read(buffer)
                .map_err(|error| format!("zstd: {error:?}"))?;
            if read > 0 || buffer.is_empty() {
                return Ok(read);
            }
            // The frame is done; another may follow.
            let Some(decoder) = self.inner.take() else {
                return Ok(0);
            };
            let (source, frames) = decoder.into_parts();
            if source.at >= source.size {
                return Ok(0);
            }
            self.inner = Some(
                StreamingDecoder::new_with_decoder(source, frames)
                    .map_err(|error| format!("zstd: {error:?}"))?,
            );
        }
    }

    /// Exactly `buffer.len()` bytes.
    pub fn read_exact(&mut self, mut buffer: &mut [u8]) -> Result<(), String> {
        while !buffer.is_empty() {
            match self.read(buffer)? {
                0 => return Err("the compressed data ends early".into()),
                read => buffer = &mut buffer[read..],
            }
        }
        Ok(())
    }
}

/// Whether the file starts as zstd does.
pub fn is_zstd(path: &str) -> bool {
    let Ok(file) = File::locked(path) else {
        return false;
    };
    let mut start = [0_u8; 4];
    matches!(file.read(&mut start), Ok(4)) && start == MAGIC
}

/// Writes `from` decompressed (or as it is, when it is not zstd) into a new
/// file `to`, flushed; the written file's size and SHA-256.
/// `progress(bytes out)`; `false` from it stops.
pub fn unpack(
    from: &str,
    to: &str,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<FileId, Option<String>> {
    let compressed = is_zstd(from);
    let input = File::locked(from).map_err(|code| Some(win::error_text(code)))?;
    let output = File::create(to).map_err(|code| Some(win::error_text(code)))?;
    let (mut zstd, plain) = if compressed {
        (Some(Zstd::open(input).map_err(Some)?), None)
    } else {
        (None, Some(input))
    };
    let mut hash = Sha256::new();
    let mut buffer = alloc::vec![0_u8; 1 << 20];
    let mut size = 0_u64;
    loop {
        let read = match (&mut zstd, &plain) {
            (Some(zstd), _) => zstd.read(&mut buffer).map_err(Some)?,
            (None, Some(file)) => file
                .read(&mut buffer)
                .map_err(|code| Some(win::error_text(code)))?,
            (None, None) => 0,
        };
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        output
            .write(&buffer[..read])
            .map_err(|code| Some(win::error_text(code)))?;
        size += read as u64;
        if !progress(size) {
            return Err(None);
        }
    }
    output.flush().map_err(|code| Some(win::error_text(code)))?;
    Ok(FileId {
        sha256: hash.finish(),
        size,
    })
}
