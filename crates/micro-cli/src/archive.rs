//! A ZIP archive built in memory, for a handful of files that are already in memory.

use flate2::write::DeflateEncoder;
use flate2::Compression;
use std::io;
use std::io::Write;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_DIRECTORY: u32 = 0x0605_4b50;
/// ZIP 2.0, the first version with deflate, which is all these archives need.
const VERSION: u16 = 20;
/// Names are UTF-8.
const UTF8_NAMES: u16 = 0x0800;
const DEFLATED: u16 = 8;

/// One file to put in an archive.
pub struct ArchiveFile {
    pub name: String,
    pub contents: Vec<u8>,
}

impl ArchiveFile {
    pub fn new(name: impl Into<String>, contents: impl Into<Vec<u8>>) -> Self {
        ArchiveFile {
            name: name.into(),
            contents: contents.into(),
        }
    }
}

/// A calendar date and time in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl UtcTime {
    pub fn of(time: SystemTime) -> Self {
        let seconds = time
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
        let (year, month, day) = civil_from_days(days);
        UtcTime {
            year,
            month,
            day,
            hour: (of_day / 3_600) as u32,
            minute: (of_day % 3_600 / 60) as u32,
            second: (of_day % 60) as u32,
        }
    }

    /// The RFC 3339 form, such as `2026-10-02T09:30:00Z`.
    pub fn rfc3339(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// The time and date as MS-DOS writes them, which is what ZIP headers carry. DOS dates start
    /// in 1980, so anything earlier is written as its first moment.
    fn dos(&self) -> (u16, u16) {
        if self.year < 1980 {
            return (0, (1 << 5) | 1);
        }
        let time = (self.hour << 11) | (self.minute << 5) | (self.second / 2);
        let date = (((self.year - 1980).min(127) as u32) << 9) | (self.month << 5) | self.day;
        (time as u16, date as u16)
    }
}

/// The proleptic Gregorian date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The bytes of a ZIP archive holding `files`, each deflated and stamped with `modified`.
pub fn zip(files: &[ArchiveFile], modified: SystemTime) -> io::Result<Vec<u8>> {
    let (time, date) = UtcTime::of(modified).dos();
    let mut archive = Vec::new();
    let mut directory = Vec::new();

    for file in files {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&file.contents)?;
        let compressed = encoder.finish()?;

        let name = file.name.as_bytes();
        let crc = crc32fast::hash(&file.contents);
        let offset = fits(archive.len(), "the archive")?;
        let packed = fits(compressed.len(), &file.name)?;
        let size = fits(file.contents.len(), &file.name)?;
        let name_length = u16::try_from(name.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name too long"))?;

        put32(&mut archive, LOCAL_HEADER);
        put16(&mut archive, VERSION);
        put16(&mut archive, UTF8_NAMES);
        put16(&mut archive, DEFLATED);
        put16(&mut archive, time);
        put16(&mut archive, date);
        put32(&mut archive, crc);
        put32(&mut archive, packed);
        put32(&mut archive, size);
        put16(&mut archive, name_length);
        put16(&mut archive, 0);
        archive.extend_from_slice(name);
        archive.extend_from_slice(&compressed);

        put32(&mut directory, CENTRAL_HEADER);
        put16(&mut directory, VERSION);
        put16(&mut directory, VERSION);
        put16(&mut directory, UTF8_NAMES);
        put16(&mut directory, DEFLATED);
        put16(&mut directory, time);
        put16(&mut directory, date);
        put32(&mut directory, crc);
        put32(&mut directory, packed);
        put32(&mut directory, size);
        put16(&mut directory, name_length);
        put16(&mut directory, 0);
        put16(&mut directory, 0);
        put16(&mut directory, 0);
        put16(&mut directory, 0);
        put32(&mut directory, 0);
        put32(&mut directory, offset);
        directory.extend_from_slice(name);
    }

    let count = u16::try_from(files.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many files"))?;
    let directory_offset = fits(archive.len(), "the archive")?;
    let directory_size = fits(directory.len(), "the archive's directory")?;
    archive.extend_from_slice(&directory);

    put32(&mut archive, END_OF_DIRECTORY);
    put16(&mut archive, 0);
    put16(&mut archive, 0);
    put16(&mut archive, count);
    put16(&mut archive, count);
    put32(&mut archive, directory_size);
    put32(&mut archive, directory_offset);
    put16(&mut archive, 0);
    Ok(archive)
}

/// A size or offset as the 32 bits a plain ZIP header has room for.
fn fits(value: usize, what: &str) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} is too large for a ZIP archive"),
        )
    })
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Reading an archive back, for tests that check what was written.
#[cfg(test)]
pub mod reading {
    use super::CENTRAL_HEADER;
    use super::END_OF_DIRECTORY;
    use super::LOCAL_HEADER;
    use flate2::read::DeflateDecoder;
    use std::io::Read;

    fn read16(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    fn read32(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    /// Every file listed in the central directory, read back through its local header.
    pub fn unzip(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
        let end = archive.len() - 22;
        assert_eq!(read32(archive, end), END_OF_DIRECTORY);
        let count = read16(archive, end + 10) as usize;
        let mut at = read32(archive, end + 16) as usize;

        let mut files = Vec::new();
        for _ in 0..count {
            assert_eq!(read32(archive, at), CENTRAL_HEADER);
            let crc = read32(archive, at + 16);
            let name_length = read16(archive, at + 28) as usize;
            let local = read32(archive, at + 42) as usize;
            let name = String::from_utf8(archive[at + 46..at + 46 + name_length].to_vec()).unwrap();
            at += 46 + name_length;

            assert_eq!(read32(archive, local), LOCAL_HEADER);
            let packed = read32(archive, local + 18) as usize;
            let start = local + 30 + read16(archive, local + 26) as usize;
            let mut contents = Vec::new();
            DeflateDecoder::new(&archive[start..start + packed])
                .read_to_end(&mut contents)
                .unwrap();
            assert_eq!(crc32fast::hash(&contents), crc, "{name}");
            files.push((name, contents));
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn an_archive_holds_what_was_put_in_it() {
        let files = vec![
            ArchiveFile::new("report.json", "{\"id\":1}\n"),
            ArchiveFile::new("session.jsonl", "line\n".repeat(1_000)),
        ];
        let archive = zip(&files, SystemTime::now()).unwrap();

        let read = reading::unzip(&archive);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0], ("report.json".into(), b"{\"id\":1}\n".to_vec()));
        assert_eq!(read[1].0, "session.jsonl");
        assert_eq!(read[1].1, "line\n".repeat(1_000).into_bytes());
    }

    #[test]
    fn dates_are_counted_from_the_unix_epoch() {
        let moment = UNIX_EPOCH + Duration::from_secs(1_790_933_400);
        let time = UtcTime::of(moment);
        assert_eq!(time.rfc3339(), "2026-10-02T09:30:00Z");
        assert_eq!(UtcTime::of(UNIX_EPOCH).rfc3339(), "1970-01-01T00:00:00Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(UtcTime::of(leap).rfc3339(), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn a_dos_date_packs_the_calendar() {
        let time = UtcTime::of(UNIX_EPOCH + Duration::from_secs(1_790_933_400));
        let (clock, date) = time.dos();
        assert_eq!(date >> 9, 2026 - 1980);
        assert_eq!((date >> 5) & 0x0f, 10);
        assert_eq!(date & 0x1f, 2);
        assert_eq!(clock >> 11, 9);
        assert_eq!((clock >> 5) & 0x3f, 30);
    }
}
