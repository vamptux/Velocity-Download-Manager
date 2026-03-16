use std::fs::File;
use std::io;

use fs2::FileExt as Fs2FileExt;

#[cfg(unix)]
use std::os::unix::fs::FileExt;

#[cfg(windows)]
use std::os::windows::fs::FileExt;

/// Zero-cost OS allocation to prevent fragmentation.
/// Uses the platform allocation fast path exposed by fs2 and only falls back
/// to `set_len` when the filesystem does not support reservation directly.
pub fn allocate_file(file: &File, size: u64) -> io::Result<()> {
    if size == 0 {
        return Ok(());
    }

    match Fs2FileExt::allocate(file, size) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => file.set_len(size),
        Err(error) => Err(error),
    }
}

/// Lock-free concurrent write API.
/// Writes directly to the byte offset without locking the file on disk.
#[inline(always)]
pub fn write_at_offset(file: &File, buffer: &[u8], offset: u64) -> io::Result<()> {
    let mut written = 0usize;
    while written < buffer.len() {
        let next_offset = offset.saturating_add(written as u64);
        let slice = &buffer[written..];
        let bytes_written = write_at_offset_once(file, slice, next_offset)?;
        if bytes_written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "write_at_offset wrote zero bytes",
            ));
        }
        written = written.saturating_add(bytes_written);
    }

    Ok(())
}

#[inline(always)]
fn write_at_offset_once(file: &File, buffer: &[u8], offset: u64) -> io::Result<usize> {
    #[cfg(windows)]
    {
        file.seek_write(buffer, offset)
    }

    #[cfg(unix)]
    {
        file.write_at(buffer, offset)
    }

    #[cfg(not(any(windows, unix)))]
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = file.try_clone()?;
        f.seek(SeekFrom::Start(offset))?;
        f.write(buffer)
    }
}
